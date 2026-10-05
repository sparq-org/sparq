//! [OPUS-4.8] sq-rh4gu (epic sq-pbz04) — a **RIF-Core** front-end over the N3
//! forward-chaining rule engine.
//!
//! RIF-Core is the W3C *Rule Interchange Format* **Core** dialect — the
//! **monotone Horn-rule** common subset of RIF-BLD (Basic Logic Dialect) and
//! RIF-PRD (Production Rule Dialect). This module accepts a RIF-Core rule set as
//! a faithful *in-engine representation* (the `Document` / `Rule` / `Atom` /
//! `Term` types below), **validates** it (range-restriction / DATALOG safety),
//! and **lowers** it to the existing N3 rule model so the proven
//! `reason_n3` forward chainer computes its closure. Reasoning is engaged only
//! behind the opt-in `rif-core` cargo feature — the lean default build carries
//! zero RIF code.
//!
//! ## Scope — RIF-Core, the MONOTONE Horn subset (honest boundary)
//!
//! This is **RIF-Core**, not full RIF-BLD or RIF-PRD. Specifically:
//!
//! * **Conditions**: a conjunction (`And`) of positive atoms — frame (`o[p->v]`),
//!   membership (`o # c`), subclass (`c1 ## c2`) atoms,
//!   plus **externally-defined builtin** calls ([`Builtin`]). `Equal` (`a = b`)
//!   atoms are permitted in rule BODIES only (see Equal-atom semantics below).
//!   No `Or`, no existentials in the body beyond the implicit universal closure,
//!   no negation.
//! * **Rules**: `Forall ?v… ( head :- body )` Horn implications with a single
//!   (conjunctive) head. The head's atoms and variables are **range-restricted**
//!   by the body (validated — see below).
//! * **Facts**: ground atoms (a rule with an empty/`True` body).
//!
//! ## Equal-atom semantics — body-only, resolved at COMPILE TIME by substitution
//!
//! [SONNET-4.6] sq-pbz04.5.4 / [OPUS-4.8] sq-26vwp — RIF-Core is an important
//! special case for `Equal` (`a = b`) atoms: unlike RIF-BLD (which permits equality
//! in heads and applies **congruence closure** / equality-propagating semantics),
//! **RIF-Core forbids `Equal` in rule conclusions** and gives body equality
//! **ground-identity** semantics — two terms are equal iff they are identical after
//! variable substitution (structural/syntactic equality over RDF terms), and `t = t`
//! is ALWAYS true (reflexivity).
//!
//! Body `Equal` atoms are resolved **at validate / lower time by
//! substitution / unification** (`resolve_body_equalities`), NOT by matching an
//! `owl:sameAs` triple at runtime. Equality is a *built-in identity relation*; it is
//! deliberately kept SEPARATE from the `owl:sameAs` RDF vocabulary — conflating the
//! two is unsound (an asserted `owl:sameAs` *data* triple must not license a RIF
//! equality, and a genuine `?x = ?y` must not depend on an `owl:sameAs` assertion).
//! The following rewrites are applied to a **fixpoint**, so NO `Equal` atom ever
//! reaches N3 lowering:
//!
//! 1. **`t = t` (identical after substitution)** — trivially true; the atom is
//!    ELIMINATED and contributes NO bindings for range-restriction. A head variable
//!    bound SOLELY by a `?x = ?x` atom is therefore rejected `UnboundHeadVar`
//!    (unifying a variable with itself binds nothing).
//! 2. **`?x = t` (one side a variable, the other any term)** — `t` is SUBSTITUTED
//!    for `?x` throughout the rule (head + body). The variable becomes
//!    **bound-by-substitution**: `?x # C :- ?x = <a>` derives `<a> # C`
//!    unconditionally, and the UnboundHeadVar sweep correctly treats `?x` as bound.
//! 3. **`?x = ?y` (two distinct variables)** — UNIFIED: one name is renamed to the
//!    other throughout the rule, so the two occurrences collapse to one variable and
//!    the join naturally requires the SAME node. `?x # Self :- ?x mgr ?y , ?x = ?y`
//!    fires exactly for the individuals that manage themselves — with no `owl:sameAs`
//!    assertion needed (reflexivity honoured) and without over-firing on distinct
//!    nodes that merely carry an `owl:sameAs` triple.
//! 4. **`t1 = t2` (two distinct NON-variable terms)** — [SONNET-4.6] sq-anyad:
//!    decided by **NUMERIC value-space equality** when BOTH sides are well-formed
//!    numeric literals, via the SHARED substrate comparator
//!    `sparq_substrate::numeric::Num::cmp_relational` (sq-v5evr, issue #1646 —
//!    the same comparator this crate's `compare`, `datalog` and `dtype` paths
//!    already drive, so the RIF front-end cannot diverge from them). Across
//!    the XSD numeric tiers `"1"^^xsd:integer = "1.0"^^xsd:decimal` is TRUE, so
//!    the atom is ELIMINATED exactly like `t = t`; `"1"^^xsd:integer =
//!    "2"^^xsd:integer` is FALSE, which makes the body UNSATISFIABLE — the rule
//!    is still range-restriction checked and then dropped at lowering, deriving
//!    nothing (vacuously true, and monotone: it could never have contributed a
//!    fact).
//!    The shared tower is exact only within an `i128` mantissa, so a WELL-FORMED
//!    `xsd:integer`/`xsd:decimal` beyond that range is classified out of it. Such a
//!    pair is NOT left undecidable: when both sides are exact-tier lexicals they are
//!    compared EXACTLY by `sparq_substrate::numeric::cmp_plain_decimal` (string
//!    arithmetic, arbitrary precision, no `f64` promotion), so
//!    `"+1<39 digits>" = "1<39 digits>"` is TRUE and an unequal huge pair is vacuous.
//!    [OPUS-5]
//!    Everything else stays fail-closed with [`RifError::DistinctGroundEqual`] —
//!    the NON-numeric literal-equality half (`pred:boolean-equal`,
//!    `pred:literal-not-identical` over strings/booleans/dates), an IRI or `List`
//!    operand, an ill-formed numeric lexical — including a lexical outside the value
//!    space of its declared derived integer datatype, so `"-1"^^xsd:positiveInteger`
//!    and `"128"^^xsd:byte` are REFUSED rather than decided — a `NaN` operand (for which
//!    `cmp_relational` reports a type error rather than a verdict), and an
//!    out-of-tower exact-tier value paired with an `xsd:float`/`xsd:double` operand
//!    (which would need the float's exact decimal expansion). Those stay deferred;
//!    the front-end refuses rather than answering incorrectly.
//!    Because substitution runs to a fixpoint
//!    FIRST, a chained `?x = ?y , ?y = "2"^^xsd:decimal` reduces both sides to the
//!    constant and the ground-identity / value-space checks are re-run on the
//!    substituted terms.
//!
//! `Equal` in a HEAD is rejected with [`RifError::EqualInConclusion`] (a Core
//! syntactic restriction relative to RIF-BLD).
//!
//! **EXPLICITLY EXCLUDED** (RIF-Core is monotone, so these would break
//! monotonicity and are *not* in the dialect): negation-as-failure / `Naf`, the
//! RIF-PRD production constructs (`Assert`/`Retract`/`Modify` actions, conflict
//! resolution, the `Negation` strong negation of RIF-BLD), function symbols in
//! the *logic* sense (uninterpreted functions / `Frame`-nesting beyond what Core
//! allows), and aggregation. This front-end **rejects** any attempt to use a
//! negative or nonmonotonic construct rather than silently mis-evaluating it.
//! Because every rule is a positive Horn implication and the chainer only ever
//! *adds* facts, the closure is **monotone**: adding facts to the input can only
//! add conclusions, never retract one. See [`Document::closure`].
//!
//! ## Builtins (RIF-Core's `func:`/`pred:` externals) — with SAFETY enforced
//!
//! RIF-Core imports a standard set of datatype / numeric / string / list
//! builtins from XPath/XQuery 1.0 Functions & Operators (the
//! `rif-builtin-function`/`rif-builtin-predicate` IRIs). [`Builtin`] models the
//! subset this front-end implements; each lowers to the equivalent N3
//! `math:`/`string:`/`list:` builtin the chainer already evaluates. **Builtin
//! safety / range-restriction is enforced**: a builtin may compute a value into a
//! variable only when its *inputs* are range-restricted (bound by a positive body
//! atom), and a head variable must likewise be bound by a positive body atom.
//! [`Document::validate`] **rejects** an unsafe rule (returning [`RifError`])
//! instead of letting the chainer loop or over-derive on unbound builtin inputs.
//!
//! Unimplemented RIF features are reported as documented out-of-scope (the
//! [`UNIMPLEMENTED`] list), never faked.

use crate::n3::Term as N3Term;
use sparq_core::dict::{Dict, Id};
// [SONNET-4.6] sq-anyad — the SHARED value-space numeric tower: the RIF Equal-atom
// path decides distinct-ground NUMERIC equality with the substrate's own
// `as_numeric` classifier (the same one sparq-engine's value path uses) and its
// relational comparator `Num::cmp_relational` (sq-v5evr, #1646 — already driven by
// this crate's `compare`, `datalog` and `dtype` paths), so no second numeric
// comparison core enters the reasoner. The `numeric` slice is a NON-optional dep of
// this crate (see Cargo.toml), so this pulls in no new dependency and no new feature.
use sparq_substrate::numeric::{as_numeric, cmp_plain_decimal, Num};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// RIF datatype / standard-vocabulary IRIs used when lowering.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
/// The one EXACT non-integer numeric tier — see [`exact_decimal_lexical`]. [OPUS-5]
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
/// N3 builtin namespaces the RIF builtins lower onto (the chainer recognises these).
const MATH: &str = "http://www.w3.org/2000/10/swap/math#";
const STRING: &str = "http://www.w3.org/2000/10/swap/string#";
const LIST: &str = "http://www.w3.org/2000/10/swap/list#";

/// RIF features deliberately NOT implemented by this Core front-end — reported as
/// documented out-of-scope (never faked into a passing test). RIF-Core is the
/// monotone Horn subset, so the nonmonotonic / production / non-Core items here
/// are not even *in* the dialect; the rest are larger-dialect surface tracked for
/// honesty. Cross-referenced by the expressivity suite so the gaps are legible.
// [SONNET-4.6] sq-pbz04.5.2 — deferral ledger (§3.2) added below.
pub const UNIMPLEMENTED: &[&str] = &[
    // NONMONOTONIC / NON-CORE by dialect (would break monotonicity — excluded by design):
    "negation-as-failure (Naf) — RIF-Core is monotone; NAF is not in the dialect",
    "RIF-BLD strong negation (Negation)",
    "RIF-PRD actions (Assert/Retract/Modify) + conflict resolution",
    "aggregation (min/max/count/sum over a group)",
    // LARGER-DIALECT logic surface, tracked-not-implemented:
    "uninterpreted function symbols / nested function terms (RIF-BLD)",
    "disjunction (Or) in rule bodies",
    "the RIF/XML presentation-syntax importer (this front-end takes the in-engine model)",
    // DEFERRED BUILTINS — soundly not mapped, each with its precise reason (§3.2):
    // [SONNET-4.6] sq-pbz04.5.2
    "func:numeric-integer-divide DEFERRED: F&O op:numeric-integer-divide truncates \
     toward zero; EYE/N3 math:integerQuotient is floor division. They diverge on any \
     negative operand (−7 idiv 2 = −3 vs floor(−3.5) = −4). A truncating N3-side \
     builtin or RIF-side compilation is needed — deferred.",
    "func:numeric-mod DEFERRED: F&O op:numeric-mod result follows dividend sign; \
     math:remainder is integer-only with divisor-sign semantics. Mixed-sign operands \
     diverge — deferred.",
    "pred:matches DEFERRED: XPath/XSD regex dialect ≠ Rust regex crate dialect \
     (e.g. XSD character-class subtraction is not supported). A mapping cannot fail \
     closed on dialect-divergent patterns without a real XSD-regex front-end — deferred.",
    // [SONNET-4.6] sq-pbz04.5.4 / [OPUS-4.8] sq-26vwp — Equal-atom handling:
    // Equal in a rule CONCLUSION is REJECTED (RIF-Core syntactic restriction) via
    // RifError::EqualInConclusion. Body Equal is resolved at compile time by
    // substitution/unification (t=t eliminated; ?x=t substituted; ?x=?y unified).
    // ONLY distinct-GROUND value-equality remains deferred (fail-closed), below.
    // [SONNET-4.6] sq-anyad — the NUMERIC half is now IMPLEMENTED (sq-v5evr landed);
    // only the non-numeric literal-equality half remains deferred.
    "pred:boolean-equal / pred:literal-not-identical / equality of distinct value-equal \
     NON-NUMERIC GROUND constants DEFERRED (fail-closed): needs value-space equality \
     beyond the numeric tower (booleans, strings, dates) — rejects with \
     DistinctGroundEqual (sq-anyad). The NUMERIC half IS implemented: distinct ground \
     numeric literals are decided by the shared substrate comparator Num::cmp_relational \
     (sq-v5evr, issue #1646), so \"1\"^^xsd:integer = \"1.0\"^^xsd:decimal holds and an \
     unequal numeric pair makes the rule vacuous; an integer/decimal beyond that tower's \
     i128 mantissa is decided EXACTLY by cmp_plain_decimal string arithmetic, and only \
     such an out-of-tower value paired with an xsd:float/xsd:double operand stays \
     fail-closed. Variable and mixed body Equal (?x=?y, \
     ?x=t) are handled soundly by compile-time substitution/unification (sq-26vwp); body \
     t=t works.",
    "guard predicates (pred:is-literal-integer etc.) DEFERRED: would require inventing \
     non-EYE N3 builtins, polluting the chainer's EYE-differential story — deferred.",
    "func:substring / func:substring-before / func:substring-after / func:string-join / \
     func:compare DEFERRED: no semantically-matching N3 target exists today \
     (string:scrape is regex capture, not substring) — deferred.",
    "list utility builtins (func:get/sublist/reverse/index-of/insert-before/remove/\
     union/distinct-values/except / pred:is-list) DEFERRED: no 1:1 N3 target; \
     multi-triple lowerings change the producer/consumer shape and its safety analysis \
     — deferred.",
    "date/time/duration builtins DEFERRED: no shared temporal tower exists (the \
     chainer's time: support is partial/EYE-shaped; sparq-substrate has no temporal \
     module) — deferred wholesale; a temporal seam is its own future design record.",
];

/// A RIF-Core term: a constant (IRI or typed literal), a universally-quantified
/// variable, or a `List` (RIF's `List(…)` term, used by the list builtins).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Term {
    /// An IRI constant (RIF `Const` of type `rif:iri`).
    Iri(String),
    /// A typed literal `"lex"^^<dt>` (RIF `Const` of an XSD type). The datatype
    /// IRI is stored explicitly; a plain string uses `xsd:string`.
    Lit {
        /// The lexical form.
        lex: String,
        /// The datatype IRI.
        datatype: String,
    },
    /// A universally-quantified variable (`?v`).
    Var(String),
    /// A RIF `List(…)` term (first-class, like N3 collections).
    List(Vec<Term>),
}

impl Term {
    /// An `xsd:integer` literal constant — convenience for facts/tests.
    pub fn int(n: i64) -> Term {
        Term::Lit { lex: n.to_string(), datatype: format!("{}integer", XSD) }
    }

    /// An `xsd:string` literal constant.
    pub fn string(s: impl Into<String>) -> Term {
        Term::Lit { lex: s.into(), datatype: format!("{}string", XSD) }
    }

    /// Is this term a variable (`?v`)?
    pub fn is_var(&self) -> bool {
        matches!(self, Term::Var(_))
    }

    /// Collect every variable name occurring in this term (recursing into lists).
    fn vars_into(&self, out: &mut BTreeSet<String>) {
        match self {
            Term::Var(v) => {
                out.insert(v.clone());
            }
            Term::List(items) => {
                for t in items {
                    t.vars_into(out);
                }
            }
            _ => {}
        }
    }

    /// Lower to the N3 term model.
    fn to_n3(&self) -> N3Term {
        match self {
            Term::Iri(i) => N3Term::Iri(i.clone()),
            Term::Lit { lex, datatype } => N3Term::Lit(lex.clone(), datatype.clone(), None),
            Term::Var(v) => N3Term::Var(v.clone()),
            Term::List(items) => N3Term::List(items.iter().map(Term::to_n3).collect()),
        }
    }
}

/// A RIF-Core **builtin** call — an externally-defined `func:`/`pred:` term.
///
/// RIF-Core imports these from XPath/XQuery 1.0 F&O. This front-end models the
/// numeric / string / list / datatype subset the expressivity suite exercises;
/// each lowers to the equivalent N3 builtin the chainer evaluates. A builtin is
/// either a **predicate** (`pred:…`, a body filter — `is_filter()`) or a
/// **function** (`func:…`, computes its last argument from the earlier ones).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Builtin {
    // --- numeric predicates (pred:numeric-*) — body filters ---
    /// `pred:numeric-equal` (a = b).
    NumericEqual,
    /// `pred:numeric-less-than` (a < b).
    NumericLessThan,
    /// `pred:numeric-greater-than` (a > b).
    NumericGreaterThan,
    /// `pred:numeric-greater-than-or-equal` (a >= b).
    NumericNotLessThan,
    /// `pred:numeric-less-than-or-equal` (a <= b).
    NumericNotGreaterThan,
    // --- numeric functions (func:numeric-*) — compute the result arg ---
    /// `func:numeric-add` (c = a + b).
    NumericAdd,
    /// `func:numeric-subtract` (c = a - b).
    NumericSubtract,
    /// `func:numeric-multiply` (c = a * b).
    NumericMultiply,
    /// `func:numeric-divide` (c = a / b).
    NumericDivide,
    // --- string predicates / functions ---
    /// `func:contains` (predicate use: a contains b).
    StringContains,
    /// `func:starts-with` (predicate use).
    StringStartsWith,
    /// `func:ends-with` (predicate use).
    StringEndsWith,
    /// `func:concat` (function: c = concat(a, b)).
    StringConcat,
    /// `func:string-length` (function: n = length(a)).
    StringLength,
    // --- list functions / predicates ---
    /// `pred:list-contains` (predicate: list contains member).
    ListContains,
    /// list length (function: n = length(list)).
    ListLength,
    // [SONNET-4.6] sq-pbz04.5.2 — 5 soundly-mapped builtins (§3.1 equivalences):
    /// `pred:numeric-not-equal` (a ≠ b) — N3 target: `math:notEqualTo`.
    /// §3.1 equivalence: both are numeric value-space inequality over the same
    /// promotion tower; the chainer's `MathNe` exists and is exercised by the N3
    /// suites.
    NumericNotEqual,
    /// `func:upper-case` (maps string to upper case) — N3 target: `string:upperCase`.
    /// §3.1 equivalence: XPath `fn:upper-case` uses default Unicode case mapping ≙
    /// Rust `str::to_uppercase` (same default, no locale/tailored mappings on either
    /// side).
    StringUpperCase,
    /// `func:lower-case` (maps string to lower case) — N3 target: `string:lowerCase`.
    /// §3.1 equivalence: symmetric to upper-case.
    StringLowerCase,
    /// `func:encode-for-uri` (RFC 3986 percent-encoding) — N3 target: `string:encodeForUri`.
    /// §3.1 equivalence: the chainer's builtin is documented as exactly XPath
    /// `fn:encode-for-uri` (RFC 3986 unreserved-set, uppercase hex) — definitionally
    /// the same function.
    StringEncodeForUri,
    /// `func:concatenate` (list concatenation) — N3 target: `list:append`.
    /// §3.1 equivalence: both concatenate a sequence of lists into one list,
    /// order-preserving; the chainer's `Append` takes `( list… )` which matches
    /// DTB's variadic signature. Variadic: `arity()` returns the MINIMUM (2 = at
    /// least one input list + one output); validate uses `>=` for this variant.
    ListConcatenate,
}

impl Builtin {
    /// `true` for the **predicate** builtins (used as a body filter, all args are
    /// inputs); `false` for the **function** builtins (the LAST argument is the
    /// computed output, the earlier ones are inputs).
    pub fn is_filter(self) -> bool {
        matches!(
            self,
            Builtin::NumericEqual
                | Builtin::NumericLessThan
                | Builtin::NumericGreaterThan
                | Builtin::NumericNotLessThan
                | Builtin::NumericNotGreaterThan
                | Builtin::NumericNotEqual // [SONNET-4.6] sq-pbz04.5.2
                | Builtin::StringContains
                | Builtin::StringStartsWith
                | Builtin::StringEndsWith
                | Builtin::ListContains
        )
    }

    /// Exact argument arity (including the result arg for function builtins).
    /// For variadic builtins (`is_variadic()` is true) this is the MINIMUM arity;
    /// validation uses `>=` instead of `==`.
    fn arity(self) -> usize {
        match self {
            // binary predicates
            Builtin::NumericEqual
            | Builtin::NumericLessThan
            | Builtin::NumericGreaterThan
            | Builtin::NumericNotLessThan
            | Builtin::NumericNotGreaterThan
            | Builtin::NumericNotEqual // [SONNET-4.6] sq-pbz04.5.2
            | Builtin::StringContains
            | Builtin::StringStartsWith
            | Builtin::StringEndsWith
            | Builtin::ListContains => 2,
            // binary-input functions producing one result
            Builtin::NumericAdd
            | Builtin::NumericSubtract
            | Builtin::NumericMultiply
            | Builtin::NumericDivide
            | Builtin::StringConcat => 3,
            // unary-input functions producing one result
            Builtin::StringLength
            | Builtin::ListLength
            | Builtin::StringUpperCase  // [SONNET-4.6] sq-pbz04.5.2
            | Builtin::StringLowerCase
            | Builtin::StringEncodeForUri => 2,
            // variadic list concat: MINIMUM arity (1 input + 1 output = 2)
            // [SONNET-4.6] sq-pbz04.5.2
            Builtin::ListConcatenate => 2,
        }
    }

    /// `true` only for the variadic-arity builtins (`func:concatenate`), where
    /// `arity()` returns the **minimum** accepted argument count and `validate`
    /// uses a `>=` check. All other builtins have a fixed arity (`==` check).
    // [SONNET-4.6] sq-pbz04.5.2
    pub fn is_variadic(self) -> bool {
        matches!(self, Builtin::ListConcatenate)
    }
}

/// A RIF-Core **condition atom** — what appears as a fact, a body conjunct, or a
/// head conclusion. (RIF-Core's positive atom forms, plus the builtin call.)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Atom {
    /// A **frame**: `obj[pred -> val]` — RIF's frame atom, the workhorse positive
    /// atom (lowers to the triple `obj pred val`).
    Frame {
        /// The frame object.
        obj: Term,
        /// The slot predicate.
        pred: Term,
        /// The slot value.
        val: Term,
    },
    /// **Membership**: `obj # class` (lowers to `obj rdf:type class`).
    Member {
        /// The member.
        obj: Term,
        /// The class.
        class: Term,
    },
    /// **Subclass**: `sub ## sup` (lowers to `sub rdfs:subClassOf sup`).
    Subclass {
        /// The subclass.
        sub: Term,
        /// The superclass.
        sup: Term,
    },
    /// **Equality**: `a = b` (body-only). Resolved at compile time by
    /// substitution/unification (`resolve_body_equalities`) — never lowered to an
    /// `owl:sameAs` triple. See the module-level Equal-atom semantics. [OPUS-4.8] sq-26vwp
    Equal {
        /// The left term.
        left: Term,
        /// The right term.
        right: Term,
    },
    /// A **builtin** call `op(args…)`. In a body: a filter (predicate) or a
    /// value-computing function (the last arg is the output). NOT allowed in a
    /// head.
    Builtin {
        /// The builtin operator.
        op: Builtin,
        /// Its arguments.
        args: Vec<Term>,
    },
}

impl Atom {
    fn vars(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        match self {
            Atom::Frame { obj, pred, val } => {
                obj.vars_into(&mut out);
                pred.vars_into(&mut out);
                val.vars_into(&mut out);
            }
            Atom::Member { obj, class } => {
                obj.vars_into(&mut out);
                class.vars_into(&mut out);
            }
            Atom::Subclass { sub, sup } => {
                sub.vars_into(&mut out);
                sup.vars_into(&mut out);
            }
            Atom::Equal { left, right } => {
                left.vars_into(&mut out);
                right.vars_into(&mut out);
            }
            Atom::Builtin { args, .. } => {
                for a in args {
                    a.vars_into(&mut out);
                }
            }
        }
        out
    }

    /// Is this a **positive (non-builtin) atom** — one that binds its variables by
    /// matching facts? Frame/Member/Subclass/Equal are positive; a builtin is not
    /// (it filters or computes, it does not generate bindings for arbitrary
    /// inputs). Used by range-restriction validation.
    fn is_positive(&self) -> bool {
        !matches!(self, Atom::Builtin { .. })
    }

    /// Lower a positive atom to one N3 triple. (Builtins lower separately, since a
    /// function builtin emits the producer/consumer ordering the chainer needs.)
    ///
    /// [OPUS-4.8] sq-26vwp — `Equal` returns `None`: body `Equal` atoms are resolved
    /// away by `resolve_body_equalities` BEFORE lowering (never emitted as an
    /// `owl:sameAs` triple) and `Equal` in a head is rejected by `validate`, so an
    /// `Equal` atom must never reach this function. A `None` here therefore fails
    /// closed at the caller rather than fabricating a `sameAs` triple.
    fn positive_to_n3(&self) -> Option<[N3Term; 3]> {
        match self {
            Atom::Frame { obj, pred, val } => Some([obj.to_n3(), pred.to_n3(), val.to_n3()]),
            Atom::Member { obj, class } => {
                Some([obj.to_n3(), N3Term::Iri(RDF_TYPE.to_string()), class.to_n3()])
            }
            Atom::Subclass { sub, sup } => {
                Some([sub.to_n3(), N3Term::Iri(RDFS_SUBCLASS_OF.to_string()), sup.to_n3()])
            }
            Atom::Equal { .. } | Atom::Builtin { .. } => None,
        }
    }
}

/// A RIF-Core **rule**: `Forall ?v… ( head :- body )`. An empty `body` makes the
/// head a (universally-closed) **fact** when ground.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    /// The conclusion atoms (a conjunctive head). Must be positive (no builtins).
    pub head: Vec<Atom>,
    /// The premise atoms (a conjunction). May contain builtins.
    pub body: Vec<Atom>,
}

impl Rule {
    /// A ground **fact** (empty body, single head atom).
    pub fn fact(head: Atom) -> Rule {
        Rule { head: vec![head], body: Vec::new() }
    }

    /// An implication `head :- body`.
    pub fn implies(head: Vec<Atom>, body: Vec<Atom>) -> Rule {
        Rule { head, body }
    }
}

/// A RIF-Core document: a set of rules (facts are rules with an empty body).
#[derive(Clone, Default, Debug)]
pub struct Document {
    /// The rules (and facts) of the document.
    pub rules: Vec<Rule>,
}

/// Why a RIF-Core document was rejected. RIF-Core requires DATALOG **safety**
/// (range-restriction); violating it is rejected up front rather than letting the
/// chainer loop or over-derive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RifError {
    /// A head variable is not bound by any positive body atom (unsafe rule — the
    /// head is not range-restricted).
    UnboundHeadVar {
        /// The offending variable name.
        var: String,
    },
    /// A builtin INPUT variable is not bound by a positive body atom (or by an
    /// earlier function builtin's output) — its input is not range-restricted, so
    /// it could not be evaluated.
    UnboundBuiltinInput {
        /// The offending variable name.
        var: String,
    },
    /// A builtin was used with the wrong arity.
    BadBuiltinArity {
        /// The builtin.
        op: Builtin,
        /// The supplied argument count.
        got: usize,
        /// The expected argument count.
        want: usize,
    },
    /// A builtin appeared in a rule HEAD (builtins are body-only).
    BuiltinInHead {
        /// The offending builtin.
        op: Builtin,
    },
    /// An `Equal` atom appeared in a rule HEAD. RIF-Core (unlike RIF-BLD) does
    /// **not** permit equality in conclusions — it is a Core syntactic restriction.
    /// `validate()` rejects any rule whose head contains an `Equal` atom with this
    /// error. [SONNET-4.6] sq-pbz04.5.4
    EqualInConclusion,
    /// A body `Equal` atom has two **distinct non-variable** terms that **numeric
    /// value-space equality cannot decide** — e.g. `"true"^^xsd:boolean =
    /// "1"^^xsd:boolean` (`pred:boolean-equal`), two distinct strings
    /// (`pred:literal-not-identical`), an IRI or `List` operand, an ill-formed
    /// numeric lexical (including a derived-integer facet violation such as
    /// `"-1"^^xsd:positiveInteger`), or a `NaN` operand (a type error, not a verdict).
    ///
    /// Two distinct grounds that ARE both well-formed numeric literals no longer
    /// reach this error: they are decided by the shared substrate comparator
    /// `Num::cmp_relational` (sq-v5evr, issue #1646) — value-equal eliminates the
    /// atom, value-unequal makes the rule vacuous. The NON-numeric literal-equality
    /// half stays fail-closed rather than answering incorrectly.
    /// [SONNET-4.6] sq-pbz04.5.4 / sq-anyad
    DistinctGroundEqual {
        /// The left-hand term (rendered as a string for display).
        left: String,
        /// The right-hand term (rendered as a string for display).
        right: String,
    },
    /// A nonmonotonic / non-Core construct was supplied (defensive — these are not
    /// representable in the [`Atom`] model, so this is reserved for importers and
    /// internal lowering invariants).
    Nonmonotonic {
        /// What was rejected.
        what: String,
    },
}

impl fmt::Display for RifError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RifError::UnboundHeadVar { var } => write!(
                f,
                "unsafe RIF-Core rule: head variable ?{} is not range-restricted \
                 (no positive body atom binds it)",
                var
            ),
            RifError::UnboundBuiltinInput { var } => write!(
                f,
                "unsafe RIF-Core rule: builtin input ?{} is not range-restricted \
                 (no positive body atom or earlier function builtin binds it)",
                var
            ),
            RifError::BadBuiltinArity { op, got, want } => write!(
                f,
                "RIF-Core builtin {:?} expects {} argument(s), got {}",
                op, want, got
            ),
            RifError::BuiltinInHead { op } => write!(
                f,
                "RIF-Core builtin {:?} may not appear in a rule head (builtins are body-only)",
                op
            ),
            // [SONNET-4.6] sq-pbz04.5.4
            RifError::EqualInConclusion => write!(
                f,
                "RIF-Core does not permit Equal (=) in a rule conclusion; use RIF-BLD \
                 for equality-in-head semantics (Core syntactic restriction)"
            ),
            // [SONNET-4.6] sq-pbz04.5.4 / sq-anyad
            RifError::DistinctGroundEqual { left, right } => write!(
                f,
                "RIF-Core body Equal with distinct ground terms ({} = {}) that NUMERIC \
                 value-space equality cannot decide (a non-numeric literal, an IRI/List \
                 operand, an ill-formed numeric lexical, or NaN); the non-numeric \
                 literal-equality half (pred:boolean-equal / pred:literal-not-identical) \
                 is deferred — sq-anyad",
                left, right
            ),
            RifError::Nonmonotonic { what } => write!(
                f,
                "RIF-Core is monotone: {} is not in the dialect (negation/production \
                 constructs are excluded)",
                what
            ),
        }
    }
}

impl std::error::Error for RifError {}

impl Document {
    /// An empty document.
    pub fn new() -> Document {
        Document::default()
    }

    /// Add a rule (or fact) to the document.
    pub fn push(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    /// **Validate** the document for RIF-Core **safety / range-restriction** and
    /// **syntactic restrictions** — the load-bearing invariant. Returns `Ok(())` if
    /// every rule is valid, else the first [`RifError`].
    ///
    /// A rule is safe when, processing the body **left to right**:
    /// * every builtin's **input** arguments are range-restricted — each input
    ///   variable is bound by an earlier positive body atom or by an earlier
    ///   function builtin's output; and
    /// * every variable in the **head** is bound by some positive body atom or a
    ///   function builtin output.
    ///
    /// In addition:
    /// * Builtins must have correct arity and may not appear in a head.
    /// * `Equal` atoms may **not** appear in a head (`RifError::EqualInConclusion`
    ///   — RIF-Core syntactic restriction; see module-level doc comment).
    /// * Body `Equal` atoms are resolved by compile-time substitution / unification
    ///   (`resolve_body_equalities`) BEFORE range-restriction, so `?x = t` binds
    ///   `?x` (substitution) while `?x = ?x` binds nothing (elimination). A body
    ///   `Equal` reducing to two **distinct non-variable** terms is decided by
    ///   numeric value-space equality when both are numeric literals (sq-anyad, via
    ///   the sq-v5evr comparator) and otherwise rejected fail-closed
    ///   (`RifError::DistinctGroundEqual`). Range-restriction is checked even for a
    ///   rule whose body is value-space UNSATISFIABLE (`1 = 2`), so an unsafe rule
    ///   is still reported rather than silently swallowed by the vacuity.
    pub fn validate(&self) -> Result<(), RifError> {
        for rule in &self.rules {
            // Head atoms must be positive and must NOT be Equal.
            // [SONNET-4.6] sq-pbz04.5.4: RIF-Core forbids Equal in conclusions.
            for h in &rule.head {
                match h {
                    Atom::Builtin { op, .. } => {
                        return Err(RifError::BuiltinInHead { op: *op });
                    }
                    Atom::Equal { .. } => {
                        return Err(RifError::EqualInConclusion);
                    }
                    _ => {}
                }
            }
            // [OPUS-4.8] sq-26vwp — resolve body `Equal` atoms by compile-time
            // substitution / unification FIRST. The returned rule has NO body `Equal`
            // atoms: each is eliminated (`t = t`), substituted (`?x = t`), or unified
            // (`?x = ?y`); a distinct-ground `t1 = t2` fails closed with
            // `DistinctGroundEqual` here (before range-restriction). Substitution DOES
            // bind — a head var bound solely by `?x = t` is now genuinely bound; a head
            // var bound solely by `?x = ?x` is now genuinely unbound (correctly caught
            // by the UnboundHeadVar sweep below).
            // [SONNET-4.6] sq-anyad — the `satisfiable` flag is deliberately IGNORED
            // here: a value-space-unsatisfiable body (`1 = 2`) still gets the full
            // range-restriction sweep, so an unsafe rule is reported loudly instead of
            // being excused by its vacuity. Only lowering acts on the flag.
            let resolved = resolve_body_equalities(rule)?.rule;
            // Walk the RESOLVED body left to right, growing the set of bound variables.
            // A positive atom binds all its variables (matched against facts). A
            // FUNCTION builtin requires its input args bound and then binds its
            // output arg; a PREDICATE builtin requires ALL its args bound.
            let mut bound: BTreeSet<String> = BTreeSet::new();
            for atom in &resolved.body {
                match atom {
                    Atom::Builtin { op, args } => {
                        // [SONNET-4.6] sq-pbz04.5.2 — variadic builtins use >= check.
                        let arity_ok = if op.is_variadic() {
                            args.len() >= op.arity()
                        } else {
                            args.len() == op.arity()
                        };
                        if !arity_ok {
                            return Err(RifError::BadBuiltinArity {
                                op: *op,
                                got: args.len(),
                                want: op.arity(),
                            });
                        }
                        if op.is_filter() {
                            // Predicate: every arg is an input — all must be bound.
                            require_bound(args, &bound)?;
                        } else {
                            // Function: all but the last arg are inputs; they must
                            // be bound. The last arg is the OUTPUT — it may be an
                            // unbound variable (which this builtin binds) or a
                            // constant/bound term (then it is a check).
                            let (inputs, output) = args.split_at(args.len() - 1);
                            require_bound(inputs, &bound)?;
                            output[0].vars_into(&mut bound);
                        }
                    }
                    // [OPUS-4.8] sq-26vwp — invariant: `resolve_body_equalities` removed
                    // every body `Equal`. Reaching this arm is an internal resolution bug;
                    // fail closed rather than silently treating it as a binding-generating
                    // positive atom (which would resurrect the old owl:sameAs behaviour).
                    Atom::Equal { .. } => {
                        return Err(RifError::Nonmonotonic {
                            what: "internal: unresolved Equal atom reached range-restriction \
                                   (resolve_body_equalities invariant violated)"
                                .to_string(),
                        });
                    }
                    positive => {
                        debug_assert!(positive.is_positive());
                        bound.append(&mut positive.vars());
                    }
                }
            }
            // Every head variable must be range-restricted by the resolved body.
            let mut head_vars = BTreeSet::new();
            for h in &resolved.head {
                head_vars.extend(h.vars());
            }
            for v in &head_vars {
                if !bound.contains(v) {
                    return Err(RifError::UnboundHeadVar { var: v.clone() });
                }
            }
        }
        Ok(())
    }

    /// Lower the (validated) document to an N3 source string. Facts become
    /// top-level triples; implications become `{ body } => { head }` N3 rules. The
    /// builtins lower to their `math:`/`string:`/`list:` N3 equivalents.
    ///
    /// Returns [`RifError`] if validation fails (so a caller cannot run an unsafe
    /// document by going straight to lowering).
    pub fn to_n3_source(&self) -> Result<String, RifError> {
        self.validate()?;
        let mut out = String::new();
        for rule in &self.rules {
            // [OPUS-4.8] sq-26vwp — lower the RESOLVED rule (body `Equal` atoms
            // substituted/unified away). A rule whose body becomes empty after
            // resolution (e.g. `?x # C :- ?x = <a>`) is an unconditional FACT.
            let resolved = resolve_body_equalities(rule)?;
            // [SONNET-4.6] sq-anyad — a body `Equal` that reduced to two numerically
            // UNEQUAL ground literals (`1 = 2`) makes the body unsatisfiable: the rule
            // is VACUOUS, so it emits nothing at all. Dropping it is exact (it could
            // never have fired) and preserves monotonicity. This check precedes the
            // empty-body case so `s # C :- 1 = 2` is NOT mistaken for a fact.
            if !resolved.satisfiable {
                continue;
            }
            let rule = resolved.rule;
            if rule.body.is_empty() {
                // Fact(s): emit each head atom as a ground triple.
                for h in &rule.head {
                    let t = h.positive_to_n3().expect("validated head atoms are positive");
                    out.push_str(&n3_triple(&t));
                    out.push_str(" .\n");
                }
            } else {
                out.push_str("{ ");
                for atom in &rule.body {
                    out.push_str(&lower_body_atom(atom));
                    out.push_str(" . ");
                }
                out.push_str("} => { ");
                for h in &rule.head {
                    let t = h.positive_to_n3().expect("validated head atoms are positive");
                    out.push_str(&n3_triple(&t));
                    out.push_str(" . ");
                }
                out.push_str("} .\n");
            }
        }
        Ok(out)
    }

    /// Compute the **monotone** forward-chaining closure of the document and
    /// return the entailed ground triples interned into `dict`. This validates,
    /// lowers to N3, and drives the existing `reason_n3` forward chainer.
    ///
    /// MONOTONE: every rule is a positive Horn implication and the chainer only
    /// ever *adds* facts, so adding input facts can only add conclusions — never
    /// retract one.
    pub fn closure(&self, dict: &mut Dict) -> Result<Vec<[Id; 3]>, RifError> {
        let src = self.to_n3_source()?;
        // The N3 engine is the proven monotone Horn chainer; reuse it wholesale.
        // A lowering bug surfaces as a parse error from a valid document, which we
        // surface as a Nonmonotonic-flavoured internal error (should never happen
        // for a validated document — covered by the unit tests).
        crate::reason_n3(dict, &src).map_err(|e| RifError::Nonmonotonic {
            what: format!("internal lowering error: {}", e),
        })
    }
}

/// Require every variable occurring in `terms` to be present in `bound`.
fn require_bound(terms: &[Term], bound: &BTreeSet<String>) -> Result<(), RifError> {
    for t in terms {
        let mut vs = BTreeSet::new();
        t.vars_into(&mut vs);
        for v in vs {
            if !bound.contains(&v) {
                return Err(RifError::UnboundBuiltinInput { var: v });
            }
        }
    }
    Ok(())
}

/// Lower a body atom (positive OR builtin) to its N3 surface form.
///
/// The caller passes atoms of a RESOLVED rule (`resolve_body_equalities`), which
/// contains NO body `Equal` atoms; a stray `Equal` would make `positive_to_n3`
/// return `None`, and the `expect` below fails closed rather than emitting an
/// unsound `owl:sameAs` triple. [OPUS-4.8] sq-26vwp
fn lower_body_atom(atom: &Atom) -> String {
    match atom {
        Atom::Builtin { op, args } => lower_builtin(*op, args),
        other => {
            let t = other.positive_to_n3().expect(
                "resolved body atom is a positive Frame/Member/Subclass (Equal atoms are \
                 resolved away before lowering; sq-26vwp)",
            );
            n3_triple(&t)
        }
    }
}

/// The outcome of resolving a rule's body `Equal` atoms. [SONNET-4.6] sq-anyad
struct Resolved {
    /// The rule with every body `Equal` removed (eliminated / substituted / unified).
    rule: Rule,
    /// `false` when a body `Equal` reduced to two ground NUMERIC literals that are
    /// value-space UNEQUAL (`1 = 2`): the body can never be satisfied, so the rule
    /// is vacuous and must not be lowered. `validate` ignores this (it still checks
    /// range-restriction); `to_n3_source` drops the rule.
    satisfiable: bool,
}

/// Resolve every body `Equal` atom of `rule` by **compile-time substitution /
/// unification**, returning an equivalent rule whose body contains NO `Equal`
/// atoms. [OPUS-4.8] sq-26vwp — this is RIF-Core's ground-identity equality done
/// at validate/lower time, NOT by matching an `owl:sameAs` triple at runtime.
///
/// The rewrites, applied to a **fixpoint** over the body's `Equal` atoms (a
/// substitution can turn `?x = ?y` into `t = t` or `t1 = t2`):
/// * `t = t` (identical after substitution) → trivially true, dropped (no binding).
/// * `?x = t` (one side a variable) → substitute `t` for `?x` throughout head+body
///   (the variable becomes bound-by-substitution).
/// * `?x = ?y` (two distinct variables) → unify: rename one to the other everywhere.
/// * `t1 = t2` (two distinct NON-variable terms) → [SONNET-4.6] sq-anyad: decided by
///   [`ground_value_equal`] when both are numeric literals — value-EQUAL drops the
///   atom like `t = t`, value-UNEQUAL marks the whole rule unsatisfiable
///   ([`Resolved::satisfiable`]); anything else is [`RifError::DistinctGroundEqual`]
///   (the non-numeric literal-equality half stays fail-closed).
///
/// An `Equal` equating a variable to a compound `List` term containing that same
/// variable is rejected (occurs-check) rather than looped.
fn resolve_body_equalities(rule: &Rule) -> Result<Resolved, RifError> {
    let mut subst: BTreeMap<String, Term> = BTreeMap::new();
    let mut satisfiable = true;
    // Iterate to a fixpoint: applying a new binding can expose a further
    // ground-identity or distinct-ground equality among the remaining atoms.
    loop {
        let mut changed = false;
        for atom in &rule.body {
            if let Atom::Equal { left, right } = atom {
                let l = apply_subst_term(left, &subst);
                let r = apply_subst_term(right, &subst);
                if l == r {
                    // Trivially true (covers ?x=?x, t=t, and already-unified pairs).
                    continue;
                }
                match (&l, &r) {
                    (Term::Var(v), _) => {
                        bind_var(&mut subst, v.clone(), r)?;
                        changed = true;
                    }
                    (_, Term::Var(v)) => {
                        bind_var(&mut subst, v.clone(), l)?;
                        changed = true;
                    }
                    _ => {
                        // Two distinct NON-variable terms. [SONNET-4.6] sq-anyad —
                        // the NUMERIC half is decided by the shared substrate
                        // comparator; everything else fails closed (never guess).
                        match ground_value_equal(&l, &r) {
                            // Value-space EQUAL across the numeric tiers (e.g.
                            // 1^^integer = 1.0^^decimal): trivially true, like `t = t`.
                            Some(true) => continue,
                            // Value-space UNEQUAL: the body can never be satisfied.
                            // Keep resolving (later atoms may still substitute) but
                            // record that the rule is vacuous.
                            Some(false) => satisfiable = false,
                            None => {
                                return Err(RifError::DistinctGroundEqual {
                                    left: format!("{:?}", l),
                                    right: format!("{:?}", r),
                                })
                            }
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    // Build the resolved rule: apply the substitution to head + non-Equal body
    // atoms, DROPPING every body Equal (all now trivially true or substituted).
    let head = rule.head.iter().map(|a| apply_subst_atom(a, &subst)).collect();
    let body = rule
        .body
        .iter()
        .filter(|a| !matches!(a, Atom::Equal { .. }))
        .map(|a| apply_subst_atom(a, &subst))
        .collect();
    Ok(Resolved { rule: Rule { head, body }, satisfiable })
}

/// The inclusive `(min, max)` VALUE-SPACE bounds of a **derived** XSD integer datatype,
/// with `None` on a side that is unbounded. The outer `None` means `datatype` carries no
/// bounding facet at all — `xsd:integer` itself (unbounded both ways), or an IRI that is
/// not an integer datatype.
///
/// XSD derives each of these from `xsd:integer` by a `minInclusive`/`maxInclusive` facet
/// pair, and a derived type's LEXICAL space is exactly the lexicals mapping into its
/// value space — so `"-1"^^xsd:positiveInteger` and `"128"^^xsd:byte` are not well-formed
/// literals of their declared datatype, however well-formed the digit string is.
///
/// This deliberately does NOT reuse `dtype::integer_subtype_ok`, which encodes the same
/// XSD table for D-entailment: that module is behind the `d-entail` feature while this
/// one is behind `rif-core` (reusing it would make `rif-core` drag in the whole
/// D-entailment module), and its signature is `i128`-bounded whereas this path must also
/// bound-check an out-of-tower lexical. The two tables are the same XSD §3.4 derivation
/// and must stay in step — folding them into one crate-internal table is tracked as
/// follow-up work rather than done here. [OPUS-5]
fn integer_facet_bounds(datatype: &str) -> Option<(Option<i128>, Option<i128>)> {
    let local = datatype.strip_prefix(XSD)?;
    Some(match local {
        "long" => (Some(i64::MIN as i128), Some(i64::MAX as i128)),
        "int" => (Some(i32::MIN as i128), Some(i32::MAX as i128)),
        "short" => (Some(i16::MIN as i128), Some(i16::MAX as i128)),
        "byte" => (Some(i8::MIN as i128), Some(i8::MAX as i128)),
        "unsignedLong" => (Some(0), Some(u64::MAX as i128)),
        "unsignedInt" => (Some(0), Some(u32::MAX as i128)),
        "unsignedShort" => (Some(0), Some(u16::MAX as i128)),
        "unsignedByte" => (Some(0), Some(u8::MAX as i128)),
        "nonNegativeInteger" => (Some(0), None),
        "positiveInteger" => (Some(1), None),
        "nonPositiveInteger" => (None, Some(0)),
        "negativeInteger" => (None, Some(-1)),
        // `xsd:integer` is unfaceted; anything else is not a derived integer datatype.
        _ => return None,
    })
}

/// Whether `lex` lies inside the value space of its DECLARED integer datatype — the
/// bounds/sign facet check neither `as_numeric` (which classifies every XSD integer
/// datatype by the same `i64`/`i128` parse) nor [`exact_decimal_lexical`] performs.
/// Without it, a facet-violating literal such as `"-1"^^xsd:positiveInteger` or
/// `"128"^^xsd:byte` would be treated as a valid numeric operand and could DECIDE an
/// `Equal` atom — contradicting the fail-closed contract for an ill-formed literal.
///
/// `true` for a datatype with no bounding facet, so the unfaceted `xsd:integer` /
/// `xsd:decimal` paths are unchanged. The comparison runs through
/// `sparq_substrate::numeric::cmp_plain_decimal`, so it is exact at ARBITRARY precision:
/// an out-of-tower lexical is bounds-checked as faithfully as a small one, and a lexical
/// that is not a plain decimal at all fails closed here too. [OPUS-5]
fn integer_facets_ok(datatype: &str, lex: &str) -> bool {
    let Some((min, max)) = integer_facet_bounds(datatype) else {
        return true;
    };
    if let Some(lo) = min {
        // `None` (not a plain decimal) fails closed alongside an under-range value.
        if !matches!(
            cmp_plain_decimal(lex, &lo.to_string()),
            Some(Ordering::Equal | Ordering::Greater)
        ) {
            return false;
        }
    }
    if let Some(hi) = max {
        if !matches!(
            cmp_plain_decimal(lex, &hi.to_string()),
            Some(Ordering::Equal | Ordering::Less)
        ) {
            return false;
        }
    }
    true
}

/// The typed numeric value of a ground RIF term, or `None` when the term is not a
/// numeric literal the tower can REPRESENT (an IRI, a `List`, a non-numeric datatype
/// such as `xsd:boolean`/`xsd:string`, an ill-formed lexical, a lexical outside the
/// value space of its declared derived integer datatype (see [`integer_facets_ok`]), an
/// unparseable datatype IRI — **or** a well-formed `xsd:integer`/`xsd:decimal` whose
/// exact value is outside the tower's `i128` mantissa; [`exact_decimal_lexical`] is the
/// fallback that keeps those decidable). Delegates to the SHARED substrate classifier
/// `sparq_substrate::numeric::as_numeric` — the substrate's public literal →
/// numeric-tower classifier, shared with sparq-engine's value path, so the RIF
/// front-end can never diverge from it on what *is* a number.
/// [SONNET-4.6] sq-anyad
fn numeric_value(t: &Term) -> Option<Num> {
    match t {
        Term::Lit { lex, datatype } => {
            if !integer_facets_ok(datatype.as_str(), lex.as_str()) {
                return None;
            }
            let dt = oxrdf::NamedNode::new(datatype.as_str()).ok()?;
            as_numeric(&oxrdf::Literal::new_typed_literal(lex.as_str(), dt))
        }
        _ => None,
    }
}

/// The plain-decimal lexical of a ground term in an **EXACT** numeric tier (an XSD
/// integer datatype or `xsd:decimal`), carrying NO magnitude bound — `None` otherwise.
///
/// This is the arbitrary-precision escape hatch for [`ground_value_equal`]: [`Num`] is
/// exact only within an `i128` mantissa, so a perfectly well-formed
/// `xsd:integer`/`xsd:decimal` beyond that range is classified out by `as_numeric` and
/// would otherwise be left undecidable. Keeping the lexical as a STRING lets
/// `sparq_substrate::numeric::cmp_plain_decimal` compare it exactly by string
/// arithmetic — no `f64` promotion, so no lossy verdict. An integer lexical must carry
/// no `.` at all (`"1.5"^^xsd:integer` is ill-formed and stays fail-closed) and must lie
/// inside the value space of its declared derived integer datatype ([`integer_facets_ok`]
/// — this fallback must not rescue a facet violation `numeric_value` just refused); the
/// digit shape itself is validated by `cmp_plain_decimal`. [OPUS-5]
fn exact_decimal_lexical(t: &Term) -> Option<&str> {
    match t {
        Term::Lit { lex, datatype } if sparq_core::is_integer_datatype(datatype.as_str()) => {
            if lex.contains('.') || !integer_facets_ok(datatype.as_str(), lex.as_str()) {
                None
            } else {
                Some(lex.as_str())
            }
        }
        Term::Lit { lex, datatype } if datatype.as_str() == XSD_DECIMAL => Some(lex.as_str()),
        _ => None,
    }
}

/// Decide a body `Equal` between two DISTINCT ground terms by **numeric value-space
/// equality**. [SONNET-4.6] sq-anyad — the numeric half of the deferral, unblocked by
/// the substrate comparator `Num::cmp_relational` (sq-v5evr, issue #1646).
///
/// * `Some(true)` / `Some(false)` — the pair was decided. Either both sides are
///   representable in the shared tower and `Num::cmp_relational` compared them across
///   the XSD numeric tiers (integer / decimal / float / double), so `"1"^^xsd:integer`
///   equals `"1.0"^^xsd:decimal` and `"1.0E0"^^xsd:double`; or — when one side is a
///   well-formed `xsd:integer`/`xsd:decimal` too large for the tower's `i128` mantissa
///   — both sides are exact-tier lexicals and
///   `sparq_substrate::numeric::cmp_plain_decimal` compared them EXACTLY by string
///   arithmetic, at arbitrary precision and with no `f64` promotion. [OPUS-5]
/// * `None` — NOT decidable here, so the caller fails closed: a non-numeric literal
///   on either side (`pred:boolean-equal`, `pred:literal-not-identical` over
///   strings / booleans / dates — the half of sq-anyad that is still deferred), an
///   IRI or `List` operand, an ill-formed numeric lexical (including one outside the
///   value space of its declared derived integer datatype, e.g.
///   `"-1"^^xsd:positiveInteger` — see [`integer_facets_ok`]), a `NaN` operand
///   (`cmp_relational` reports NaN as a type error rather than a verdict, and this
///   front-end refuses rather than choosing an answer for it), or an out-of-tower
///   exact-tier value paired with an `xsd:float`/`xsd:double` operand (deciding that
///   would need the exact decimal expansion of the binary float — a wider seam than
///   this bead, tracked rather than guessed).
fn ground_value_equal(l: &Term, r: &Term) -> Option<bool> {
    match (numeric_value(l), numeric_value(r)) {
        (Some(a), Some(b)) => a.cmp_relational(b).map(|o| o == Ordering::Equal),
        // At least one side is not tower-representable. It can still be a WELL-FORMED
        // integer/decimal beyond the `i128` mantissa — decide those exactly rather than
        // reporting a valid pair as undecidable. [OPUS-5]
        _ => {
            let a = exact_decimal_lexical(l)?;
            let b = exact_decimal_lexical(r)?;
            cmp_plain_decimal(a, b).map(|o| o == Ordering::Equal)
        }
    }
}

/// Insert `v -> t` into `subst`, keeping the map idempotent (every existing value
/// that mentions `v` is rewritten to `t`). `t` must already be resolved w.r.t.
/// `subst` (the caller applies `apply_subst_term` first). Occurs-check: a variable
/// equated to a compound term containing itself is rejected. [OPUS-4.8] sq-26vwp
fn bind_var(
    subst: &mut BTreeMap<String, Term>,
    v: String,
    t: Term,
) -> Result<(), RifError> {
    if term_contains_var(&t, &v) {
        return Err(RifError::Nonmonotonic {
            what: format!("cyclic RIF-Core equality (occurs-check failed): ?{} = {:?}", v, t),
        });
    }
    for val in subst.values_mut() {
        *val = replace_var_in_term(val, &v, &t);
    }
    subst.insert(v, t);
    Ok(())
}

/// Apply a (fully resolved, idempotent) substitution to a term — a single lookup
/// per variable suffices, recursing into `List` terms. [OPUS-4.8] sq-26vwp
fn apply_subst_term(t: &Term, subst: &BTreeMap<String, Term>) -> Term {
    match t {
        Term::Var(v) => subst.get(v).cloned().unwrap_or_else(|| t.clone()),
        Term::List(items) => {
            Term::List(items.iter().map(|x| apply_subst_term(x, subst)).collect())
        }
        other => other.clone(),
    }
}

/// Apply a substitution to every term of an atom. [OPUS-4.8] sq-26vwp
fn apply_subst_atom(a: &Atom, subst: &BTreeMap<String, Term>) -> Atom {
    let s = |t: &Term| apply_subst_term(t, subst);
    match a {
        Atom::Frame { obj, pred, val } => {
            Atom::Frame { obj: s(obj), pred: s(pred), val: s(val) }
        }
        Atom::Member { obj, class } => Atom::Member { obj: s(obj), class: s(class) },
        Atom::Subclass { sub, sup } => Atom::Subclass { sub: s(sub), sup: s(sup) },
        Atom::Equal { left, right } => Atom::Equal { left: s(left), right: s(right) },
        Atom::Builtin { op, args } => {
            Atom::Builtin { op: *op, args: args.iter().map(s).collect() }
        }
    }
}

/// Replace every occurrence of variable `v` in `t` with `replacement` (recursing
/// into `List` terms). Used to keep the substitution map idempotent. [OPUS-4.8] sq-26vwp
fn replace_var_in_term(t: &Term, v: &str, replacement: &Term) -> Term {
    match t {
        Term::Var(name) if name == v => replacement.clone(),
        Term::List(items) => {
            Term::List(items.iter().map(|x| replace_var_in_term(x, v, replacement)).collect())
        }
        other => other.clone(),
    }
}

/// Does variable `v` occur anywhere in `t` (recursing into `List` terms)? The
/// occurs-check for unification. [OPUS-4.8] sq-26vwp
fn term_contains_var(t: &Term, v: &str) -> bool {
    match t {
        Term::Var(name) => name == v,
        Term::List(items) => items.iter().any(|x| term_contains_var(x, v)),
        _ => false,
    }
}

/// Lower a builtin call to its N3 builtin triple. Predicate builtins become
/// `arg0 math:lessThan arg1` etc.; function builtins become
/// `( in0 in1 ) math:sum out`.
fn lower_builtin(op: Builtin, args: &[Term]) -> String {
    let n3 = |t: &Term| n3_term(&t.to_n3());
    match op {
        // numeric predicates
        Builtin::NumericEqual => bin(&n3(&args[0]), MATH, "equalTo", &n3(&args[1])),
        Builtin::NumericLessThan => bin(&n3(&args[0]), MATH, "lessThan", &n3(&args[1])),
        Builtin::NumericGreaterThan => bin(&n3(&args[0]), MATH, "greaterThan", &n3(&args[1])),
        Builtin::NumericNotLessThan => bin(&n3(&args[0]), MATH, "notLessThan", &n3(&args[1])),
        Builtin::NumericNotGreaterThan => bin(&n3(&args[0]), MATH, "notGreaterThan", &n3(&args[1])),
        // string predicates
        Builtin::StringContains => bin(&n3(&args[0]), STRING, "contains", &n3(&args[1])),
        Builtin::StringStartsWith => bin(&n3(&args[0]), STRING, "startsWith", &n3(&args[1])),
        Builtin::StringEndsWith => bin(&n3(&args[0]), STRING, "endsWith", &n3(&args[1])),
        // list predicate: `?list list:member ?x` — subject is the list (arg0),
        // object the candidate member (arg1).
        Builtin::ListContains => bin(&n3(&args[0]), LIST, "member", &n3(&args[1])),
        // numeric functions: ( a b ) math:sum out
        Builtin::NumericAdd => func2(&n3(&args[0]), &n3(&args[1]), MATH, "sum", &n3(&args[2])),
        Builtin::NumericSubtract => {
            func2(&n3(&args[0]), &n3(&args[1]), MATH, "difference", &n3(&args[2]))
        }
        Builtin::NumericMultiply => {
            func2(&n3(&args[0]), &n3(&args[1]), MATH, "product", &n3(&args[2]))
        }
        Builtin::NumericDivide => {
            func2(&n3(&args[0]), &n3(&args[1]), MATH, "quotient", &n3(&args[2]))
        }
        // string functions
        Builtin::StringConcat => {
            func2(&n3(&args[0]), &n3(&args[1]), STRING, "concatenation", &n3(&args[2]))
        }
        Builtin::StringLength => bin(&n3(&args[0]), STRING, "length", &n3(&args[1])),
        // list function: list list:length out
        Builtin::ListLength => bin(&n3(&args[0]), LIST, "length", &n3(&args[1])),
        // [SONNET-4.6] sq-pbz04.5.2 — 5 new soundly-mapped builtins:
        // pred:numeric-not-equal: a math:notEqualTo b
        Builtin::NumericNotEqual => bin(&n3(&args[0]), MATH, "notEqualTo", &n3(&args[1])),
        // func:upper-case: s string:upperCase out
        Builtin::StringUpperCase => bin(&n3(&args[0]), STRING, "upperCase", &n3(&args[1])),
        // func:lower-case: s string:lowerCase out
        Builtin::StringLowerCase => bin(&n3(&args[0]), STRING, "lowerCase", &n3(&args[1])),
        // func:encode-for-uri: s string:encodeForUri out
        Builtin::StringEncodeForUri => bin(&n3(&args[0]), STRING, "encodeForUri", &n3(&args[1])),
        // func:concatenate (variadic lists): ( L1 L2 ... ) list:append out
        Builtin::ListConcatenate => {
            let n_inputs = args.len() - 1;
            let input_terms: Vec<String> = args[..n_inputs].iter().map(n3).collect();
            format!("( {} ) <{}append> {}", input_terms.join(" "), LIST, n3(&args[n_inputs]))
        }
    }
}

/// `s <ns#local> o`.
fn bin(s: &str, ns: &str, local: &str, o: &str) -> String {
    format!("{} <{}{}> {}", s, ns, local, o)
}

/// `( a b ) <ns#local> out` — a binary functional builtin.
fn func2(a: &str, b: &str, ns: &str, local: &str, out: &str) -> String {
    format!("( {} {} ) <{}{}> {}", a, b, ns, local, out)
}

/// Render one N3 triple `s p o`.
fn n3_triple(t: &[N3Term; 3]) -> String {
    format!("{} {} {}", n3_term(&t[0]), n3_term(&t[1]), n3_term(&t[2]))
}

/// Render an N3 term to its surface syntax (IRIs in `<…>`, vars as `?v`, literals
/// `"lex"^^<dt>`, lists `( … )`).
fn n3_term(t: &N3Term) -> String {
    match t {
        N3Term::Iri(i) => format!("<{}>", i),
        N3Term::Var(v) => format!("?{}", v),
        N3Term::Lit(lex, dt, lang) => match lang {
            Some(l) => format!("\"{}\"@{}", escape_lit(lex), l),
            None => format!("\"{}\"^^<{}>", escape_lit(lex), dt),
        },
        N3Term::Blank(b) => format!("_:{}", b),
        N3Term::List(items) => {
            let inner: Vec<String> = items.iter().map(n3_term).collect();
            format!("( {} )", inner.join(" "))
        }
        // A nested formula cannot appear in a lowered RIF-Core document.
        N3Term::Formula(_) => "{ }".to_string(),
        // RIF-Core has no frame/term form that lowers to an RDF-star quoted
        // triple, but render one faithfully if it ever flows through. [FABLE-5]
        N3Term::Triple(tr) => {
            format!("<< {} {} {} >>", n3_term(&tr[0]), n3_term(&tr[1]), n3_term(&tr[2]))
        }
    }
}

/// Escape a literal lexical form for N3 double-quoted strings.
fn escape_lit(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{NamedNode, Term as OT};
    use sparq_core::dict::Dict;

    fn iri(s: &str) -> Term {
        Term::Iri(s.to_string())
    }
    fn var(s: &str) -> Term {
        Term::Var(s.to_string())
    }

    /// Id of an IRI in `dict` (0 if absent) — mirrors the n3 module's test helper.
    fn id(dict: &Dict, i: &str) -> Id {
        dict.lookup(&OT::NamedNode(NamedNode::new_unchecked(i.to_string())))
    }

    /// Does the closure contain the ground IRI triple (s, p, o)?
    fn has(dict: &Dict, closure: &[[Id; 3]], s: &str, p: &str, o: &str) -> bool {
        let (a, b, c) = (id(dict, s), id(dict, p), id(dict, o));
        a != 0 && b != 0 && c != 0 && closure.contains(&[a, b, c])
    }

    /// The lexical value of an `xsd:integer` object in an (s, p, ?) frame.
    fn int_object(dict: &Dict, closure: &[[Id; 3]], s: &str, p: &str) -> Option<String> {
        use oxrdf::Term as OxT;
        let (a, b) = (id(dict, s), id(dict, p));
        closure.iter().find(|t| t[0] == a && t[1] == b).map(|t| match dict.term(t[2]) {
            OxT::Literal(l) => l.value().to_string(),
            other => other.to_string(),
        })
    }

    #[test]
    fn term_constructors_and_vars() {
        assert_eq!(
            Term::int(7),
            Term::Lit { lex: "7".into(), datatype: format!("{}integer", XSD) }
        );
        assert_eq!(
            Term::string("hi"),
            Term::Lit { lex: "hi".into(), datatype: format!("{}string", XSD) }
        );
        assert!(var("x").is_var());
        assert!(!iri("u").is_var());
        let mut vs = BTreeSet::new();
        Term::List(vec![var("a"), iri("u"), var("b")]).vars_into(&mut vs);
        assert_eq!(vs, ["a".to_string(), "b".to_string()].into_iter().collect());
    }

    #[test]
    fn builtin_arity_and_filter_flags() {
        assert!(Builtin::NumericLessThan.is_filter());
        assert!(!Builtin::NumericAdd.is_filter());
        assert_eq!(Builtin::NumericAdd.arity(), 3);
        assert_eq!(Builtin::NumericLessThan.arity(), 2);
        assert_eq!(Builtin::StringLength.arity(), 2);
    }

    #[test]
    fn atom_vars_and_positive() {
        let f = Atom::Frame { obj: var("o"), pred: iri("p"), val: var("v") };
        assert!(f.is_positive());
        assert_eq!(f.vars(), ["o".to_string(), "v".to_string()].into_iter().collect());
        let b = Atom::Builtin { op: Builtin::NumericLessThan, args: vec![var("a"), var("b")] };
        assert!(!b.is_positive());
    }

    #[test]
    fn rule_constructors() {
        let fact = Rule::fact(Atom::Member { obj: iri("a"), class: iri("C") });
        assert!(fact.body.is_empty());
        let r = Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("C") }],
            vec![Atom::Member { obj: var("x"), class: iri("D") }],
        );
        assert_eq!(r.body.len(), 1);
    }

    #[test]
    fn document_new_and_push() {
        let mut d = Document::new();
        assert!(d.rules.is_empty());
        d.push(Rule::fact(Atom::Member { obj: iri("a"), class: iri("C") }));
        assert_eq!(d.rules.len(), 1);
    }

    /// The load-bearing MONOTONE Horn closure over frame + membership.
    #[test]
    fn closure_propagates_membership() {
        // Forall ?x ( ?x # C :- ?x # D ) ; a # D.
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Member { obj: iri("http://ex/a"), class: iri("http://ex/D") }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/C") }],
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/D") }],
        ));
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("safe document");
        assert!(
            has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
            "a is a C, derived from the rule + a # D"
        );
    }

    /// Monotonicity: adding a fact only ADDS conclusions, never retracts.
    #[test]
    fn closure_is_monotone() {
        let rule = Rule::implies(
            vec![Atom::Frame { obj: var("x"), pred: iri("http://ex/q"), val: var("y") }],
            vec![Atom::Frame { obj: var("x"), pred: iri("http://ex/p"), val: var("y") }],
        );
        let base_fact = Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri("http://ex/p"),
            val: iri("http://ex/b"),
        });

        let mut doc1 = Document::new();
        doc1.push(rule.clone());
        doc1.push(base_fact.clone());
        let mut d1 = Dict::new();
        let c1 = doc1.closure(&mut d1).unwrap();
        // a q b is derived.
        assert!(has(&d1, &c1, "http://ex/a", "http://ex/q", "http://ex/b"));

        // Add a SECOND fact; the closure must derive its consequence TOO (monotone:
        // superset, plus a new conclusion).
        let mut doc2 = doc1.clone();
        doc2.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/c"),
            pred: iri("http://ex/p"),
            val: iri("http://ex/d"),
        }));
        let mut d2 = Dict::new();
        let c2 = doc2.closure(&mut d2).unwrap();
        // Everything still derivable, plus the new c q d.
        assert!(has(&d2, &c2, "http://ex/a", "http://ex/q", "http://ex/b"));
        assert!(has(&d2, &c2, "http://ex/c", "http://ex/q", "http://ex/d"));
        assert!(
            c2.len() > c1.len(),
            "adding a fact added a conclusion; never retracted one"
        );
    }

    /// A numeric FUNCTION builtin computes its output and a head can use it.
    #[test]
    fn builtin_function_computes_output() {
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri("http://ex/x"),
            val: Term::int(2),
        }));
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri("http://ex/y"),
            val: Term::int(3),
        }));
        doc.push(Rule::implies(
            vec![Atom::Frame {
                obj: iri("http://ex/a"),
                pred: iri("http://ex/sum"),
                val: var("z"),
            }],
            vec![
                Atom::Frame { obj: iri("http://ex/a"), pred: iri("http://ex/x"), val: var("x") },
                Atom::Frame { obj: iri("http://ex/a"), pred: iri("http://ex/y"), val: var("y") },
                Atom::Builtin {
                    op: Builtin::NumericAdd,
                    args: vec![var("x"), var("y"), var("z")],
                },
            ],
        ));
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).unwrap();
        assert_eq!(
            int_object(&dict, &closure, "http://ex/a", "http://ex/sum").as_deref(),
            Some("5"),
            "2 + 3 = 5"
        );
    }

    /// A numeric PREDICATE builtin filters the body.
    #[test]
    fn builtin_predicate_filters() {
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri("http://ex/age"),
            val: Term::int(20),
        }));
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/b"),
            pred: iri("http://ex/age"),
            val: Term::int(10),
        }));
        // adult(?p) :- ?p age ?n , ?n > 18
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("p"), class: iri("http://ex/Adult") }],
            vec![
                Atom::Frame { obj: var("p"), pred: iri("http://ex/age"), val: var("n") },
                Atom::Builtin {
                    op: Builtin::NumericGreaterThan,
                    args: vec![var("n"), Term::int(18)],
                },
            ],
        ));
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).unwrap();
        assert!(
            has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/Adult"),
            "a (age 20) is an Adult"
        );
        assert!(
            !has(&dict, &closure, "http://ex/b", RDF_TYPE, "http://ex/Adult"),
            "b (age 10) is NOT an Adult"
        );
    }

    #[test]
    fn validate_rejects_unbound_head_var() {
        let mut doc = Document::new();
        doc.push(Rule::implies(
            vec![Atom::Frame { obj: var("x"), pred: iri("http://ex/p"), val: var("y") }],
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/C") }],
        ));
        assert_eq!(doc.validate(), Err(RifError::UnboundHeadVar { var: "y".to_string() }));
        // And closure refuses to run it.
        let mut dict = Dict::new();
        assert!(doc.closure(&mut dict).is_err());
    }

    #[test]
    fn validate_rejects_unbound_builtin_input() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: var("p"), class: iri("http://ex/C") }],
            vec![
                Atom::Member { obj: var("p"), class: iri("http://ex/D") },
                Atom::Builtin {
                    op: Builtin::NumericGreaterThan,
                    args: vec![var("n"), Term::int(0)],
                },
            ],
        ));
        assert_eq!(d.validate(), Err(RifError::UnboundBuiltinInput { var: "n".to_string() }));
    }

    #[test]
    fn validate_rejects_builtin_in_head() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Builtin {
                op: Builtin::NumericLessThan,
                args: vec![var("a"), var("b")],
            }],
            vec![Atom::Frame { obj: var("a"), pred: iri("http://ex/p"), val: var("b") }],
        ));
        assert_eq!(d.validate(), Err(RifError::BuiltinInHead { op: Builtin::NumericLessThan }));
    }

    #[test]
    fn validate_rejects_bad_arity() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/C") }],
            vec![
                Atom::Member { obj: var("x"), class: iri("http://ex/D") },
                Atom::Builtin { op: Builtin::NumericAdd, args: vec![var("x")] },
            ],
        ));
        assert!(matches!(
            d.validate(),
            Err(RifError::BadBuiltinArity { op: Builtin::NumericAdd, got: 1, want: 3 })
        ));
    }

    // ---------------------------------------------------------- Equal-atom tests
    // [SONNET-4.6] sq-pbz04.5.4 — the three Equal-atom semantic requirements:
    // (1) Equal in conclusion is rejected (non-vacuous);
    // (2) ground-identity body Equal works (trivially true, rule fires);
    // (3) value-equality (distinct ground constants) is fail-closed-abstained.

    /// (1) Equal in a CONCLUSION — both as a fact head and as a rule head —
    /// MUST be rejected with `EqualInConclusion`. This is a non-vacuous test:
    /// it asserts the validator catches the case and that `closure` refuses it.
    #[test]
    fn validate_rejects_equal_in_conclusion() {
        // Case A: a "fact" with an Equal head (Rule::fact wraps a single head atom;
        // an empty body makes it a fact, so the head IS the conclusion).
        let mut d = Document::new();
        d.push(Rule::fact(Atom::Equal { left: iri("http://ex/a"), right: iri("http://ex/b") }));
        assert_eq!(
            d.validate(),
            Err(RifError::EqualInConclusion),
            "Equal as a fact head (conclusion) must be rejected"
        );
        let mut dict = Dict::new();
        assert!(d.closure(&mut dict).is_err(), "closure must refuse Equal-in-conclusion");

        // Case B: an implication whose head contains an Equal.
        let mut d2 = Document::new();
        d2.push(Rule::implies(
            vec![Atom::Equal { left: var("x"), right: iri("http://ex/b") }],
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/C") }],
        ));
        assert_eq!(
            d2.validate(),
            Err(RifError::EqualInConclusion),
            "Equal in a rule head must be rejected"
        );
    }

    /// (2) Ground-identity body Equal: `t = t` (syntactically identical terms) is
    /// trivially true and is ELIMINATED at lowering — the rule fires as if the
    /// atom were absent. This exercises the real path (`closure`), not a mock.
    #[test]
    fn body_equal_ground_identity_works() {
        // Rule: ?x # Adult :- ?x age ?n , ?n = ?n
        // The `?n = ?n` atom is ground-identical after substitution → eliminated.
        // The rule should fire just as if the Equal atom were not there.
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/p"),
            pred: iri("http://ex/age"),
            val: Term::int(30),
        }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/Adult") }],
            vec![
                Atom::Frame { obj: var("x"), pred: iri("http://ex/age"), val: var("n") },
                // ?n = ?n is trivially true; eliminated at lowering.
                Atom::Equal { left: var("n"), right: var("n") },
            ],
        ));
        doc.validate().expect("ground-identity body Equal is valid");
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure must succeed");
        assert!(
            has(&dict, &closure, "http://ex/p", RDF_TYPE, "http://ex/Adult"),
            "rule fires with trivially-true ground-identity body Equal eliminated"
        );
    }

    /// (2b) A body `?n=?n` that is the SOLE binder of a head variable must be
    /// REJECTED as UnboundHeadVar: the atom is eliminated at lowering, so the head
    /// variable would be genuinely unbound in the emitted N3 rule. [OPUS-4.8] sq-26vwp.
    #[test]
    fn body_equal_identity_sole_binder_rejects_unbound_head_var() {
        // Rule: ?n rdf:type C :- ?n = ?n
        // `?n` is ONLY "bound" by the ?n=?n Equal atom, which is eliminated.
        // validate() must catch this and return UnboundHeadVar { var: "n" }.
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: var("n"), class: iri("http://ex/C") }],
            vec![Atom::Equal { left: var("n"), right: var("n") }],
        ));
        assert_eq!(
            d.validate(),
            Err(RifError::UnboundHeadVar { var: "n".to_string() }),
            "head var solely bound by a ?n=?n (elided) atom must be rejected UnboundHeadVar"
        );
        let mut dict = Dict::new();
        assert!(d.closure(&mut dict).is_err(), "closure must refuse sole-binder ?n=?n");

        // Contrast: ?n=?n alongside a real binder is VALID (existing test case).
        // Rule: ?x # Adult :- ?x age ?n , ?n = ?n   → OK (Frame binds x and n).
        let mut d2 = Document::new();
        d2.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/Adult") }],
            vec![
                Atom::Frame { obj: var("x"), pred: iri("http://ex/age"), val: var("n") },
                Atom::Equal { left: var("n"), right: var("n") },
            ],
        ));
        assert_eq!(
            d2.validate(),
            Ok(()),
            "?n=?n alongside a Frame binder is valid (Frame already binds n)"
        );
    }

    // ---- [SONNET-4.6] sq-anyad — distinct-ground body Equal: the NUMERIC half is
    // decided by the shared substrate comparator (sq-v5evr / #1646); the non-numeric
    // literal-equality half (pred:boolean-equal / pred:literal-not-identical) stays
    // fail-closed. ----

    /// A `?x # C :- ?x # D , <l> = <r>` document whose body carries one distinct-ground
    /// `Equal`, plus the fact `<a> # D`. `Term::Lit` operands are built from
    /// `(lex, datatype-local-name)` pairs.
    fn distinct_ground_equal_doc(l: (&str, &str), r: (&str, &str)) -> Document {
        let lit = |(lex, dt): (&str, &str)| Term::Lit {
            lex: lex.to_string(),
            datatype: format!("{}{}", XSD, dt),
        };
        let mut d = Document::new();
        d.push(Rule::fact(Atom::Member { obj: iri("http://ex/a"), class: iri("http://ex/D") }));
        d.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/C") }],
            vec![
                Atom::Member { obj: var("x"), class: iri("http://ex/D") },
                Atom::Equal { left: lit(l), right: lit(r) },
            ],
        ));
        d
    }

    /// (3a) Distinct ground NUMERIC constants that are value-space EQUAL (`"1"^^xsd:integer
    /// = "1.0"^^xsd:decimal`) are now DECIDED — the atom is trivially true and eliminated
    /// like `t = t`, so the rule fires. Equality is across the XSD numeric tiers
    /// (integer / decimal / float / double), exactly as `Num::cmp_relational` defines it.
    #[test]
    fn body_equal_value_equal_numeric_grounds_are_eliminated_and_the_rule_fires() {
        for (l, r) in [
            (("1", "integer"), ("1.0", "decimal")),
            (("1", "integer"), ("1.0E0", "double")),
            (("2.50", "decimal"), ("2.5", "float")),
            (("-0.0", "double"), ("0", "integer")),
        ] {
            let d = distinct_ground_equal_doc(l, r);
            d.validate().unwrap_or_else(|e| {
                panic!("value-equal numeric grounds {:?} = {:?} must validate: {}", l, r, e)
            });
            let mut dict = Dict::new();
            let closure = d.closure(&mut dict).expect("closure succeeds");
            assert!(
                has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
                "{:?} = {:?} is value-space TRUE, so the rule must fire",
                l,
                r
            );
        }
    }

    /// (3b) Distinct ground NUMERIC constants that are value-space UNEQUAL (`1 = 2`) make
    /// the body unsatisfiable: the rule is VACUOUS — the document still validates, the
    /// closure still succeeds, and the head is simply never derived. The control document
    /// (same rule without the Equal) DOES derive it, so this pins the vacuity rather than
    /// a broken rule.
    #[test]
    fn body_equal_value_unequal_numeric_grounds_make_the_rule_vacuous() {
        let d = distinct_ground_equal_doc(("1", "integer"), ("2", "integer"));
        d.validate().expect("an unsatisfiable-but-safe rule is valid");
        let mut dict = Dict::new();
        let closure = d.closure(&mut dict).expect("closure succeeds");
        assert!(
            !has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
            "1 = 2 is value-space FALSE, so the rule must derive NOTHING"
        );
        // Control: the identical rule with a value-EQUAL Equal does derive the head.
        let ctrl = distinct_ground_equal_doc(("1", "integer"), ("1.00", "decimal"));
        let mut cdict = Dict::new();
        let cclosure = ctrl.closure(&mut cdict).expect("closure succeeds");
        assert!(
            has(&cdict, &cclosure, "http://ex/a", RDF_TYPE, "http://ex/C"),
            "control: the same rule fires when the ground Equal is value-space TRUE"
        );
    }

    /// (3b') A rule whose body is value-space unsatisfiable is STILL range-restriction
    /// checked — vacuity must not excuse an unsafe rule (`validate` deliberately ignores
    /// the satisfiability flag; only lowering acts on it).
    #[test]
    fn unsatisfiable_body_is_still_range_restriction_checked() {
        let lit = |lex: &str| Term::Lit {
            lex: lex.to_string(),
            datatype: format!("{}integer", XSD),
        };
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: var("y"), class: iri("http://ex/C") }],
            vec![Atom::Equal { left: lit("1"), right: lit("2") }],
        ));
        assert!(
            matches!(d.validate(), Err(RifError::UnboundHeadVar { .. })),
            "an unsafe head variable is reported even though the body is unsatisfiable"
        );
    }

    /// (3c) The still-PENDING half: distinct ground constants that numeric value-space
    /// equality cannot decide stay fail-closed with `DistinctGroundEqual` —
    /// `pred:boolean-equal` (booleans), `pred:literal-not-identical` (strings), a
    /// non-numeric datatype, and an ill-formed numeric lexical. sq-anyad does NOT guess
    /// an answer for these.
    #[test]
    fn body_equal_non_numeric_distinct_grounds_still_fail_closed() {
        for (l, r) in [
            // pred:boolean-equal — the boolean value space is NOT the numeric tower.
            (("true", "boolean"), ("1", "boolean")),
            (("true", "boolean"), ("false", "boolean")),
            // pred:literal-not-identical over strings.
            (("a", "string"), ("b", "string")),
            // Temporal literals: outside the numeric tower entirely (no shared temporal
            // value space exists in the substrate — see the UNIMPLEMENTED ledger).
            (("2026-07-31", "date"), ("2026-07-30", "date")),
            // Ill-formed numeric lexical: not a well-formed number, so not decidable.
            (("abc", "integer"), ("1", "integer")),
            // Ill-formed BEYOND the tower too: a fraction part is invalid for
            // xsd:integer, so the exact-lexical fallback must not rescue it. [OPUS-5]
            (("170141183460469231731687303715884105728.5", "integer"), ("1", "integer")),
            // Numeric datatype on one side only.
            (("1", "integer"), ("1", "string")),
            // Out-of-tower exact value vs a BINARY float tier: deciding this needs the
            // float's exact decimal expansion, which this front-end does not do — so it
            // fails closed rather than promoting the huge integer to a lossy f64. [OPUS-5]
            (("170141183460469231731687303715884105728", "integer"), ("1.0E0", "double")),
        ] {
            let d = distinct_ground_equal_doc(l, r);
            assert!(
                matches!(d.validate(), Err(RifError::DistinctGroundEqual { .. })),
                "{:?} = {:?} is not numerically decidable and must fail closed",
                l,
                r
            );
            let mut dict = Dict::new();
            assert!(d.closure(&mut dict).is_err(), "closure must refuse DistinctGroundEqual");
        }
    }

    /// (3c'') A lexical outside the VALUE SPACE of its declared derived integer datatype
    /// is not a well-formed literal of that datatype, however well-formed the digit
    /// string is — so it must fail closed rather than decide the atom. `as_numeric`
    /// classifies every XSD integer datatype by the same `i64`/`i128` parse and the
    /// exact-lexical fallback carries no bound, so this is the facet check both paths
    /// need (`integer_facets_ok`). [OPUS-5]
    #[test]
    fn body_equal_derived_integer_facet_violations_fail_closed() {
        // A magnitude past `i128`, so it also exercises the arbitrary-precision path.
        const HUGE: &str = "170141183460469231731687303715884105728";
        let huge_unsigned = (HUGE, "unsignedByte");
        for (l, r) in [
            // Sign facets on the unbounded derived types.
            (("-1", "positiveInteger"), ("-1", "integer")),
            (("0", "positiveInteger"), ("0", "integer")),
            (("-1", "nonNegativeInteger"), ("-1", "integer")),
            (("1", "nonPositiveInteger"), ("1", "integer")),
            (("0", "negativeInteger"), ("0", "integer")),
            // Fixed-width signed ranges.
            (("128", "byte"), ("128", "integer")),
            (("-129", "byte"), ("-129", "integer")),
            (("32768", "short"), ("32768", "integer")),
            // Unsigned: negative, and past the width.
            (("-1", "unsignedLong"), ("-1", "integer")),
            (("18446744073709551616", "unsignedLong"), ("18446744073709551616", "integer")),
            (("256", "unsignedByte"), ("256", "integer")),
            // Out of tower AND out of facet range: the exact-lexical fallback must not
            // rescue what the tower path refused.
            (huge_unsigned, (HUGE, "integer")),
        ] {
            let d = distinct_ground_equal_doc(l, r);
            assert!(
                matches!(d.validate(), Err(RifError::DistinctGroundEqual { .. })),
                "{:?} violates its datatype's value-space facet and must fail closed",
                l
            );
            let mut dict = Dict::new();
            assert!(d.closure(&mut dict).is_err(), "closure must refuse a facet violation");
        }
    }

    /// (3c''') The controls for the facet check: the INCLUSIVE boundary values of each
    /// derived integer datatype are valid, so they are still DECIDED against the
    /// equal-valued `xsd:integer` and the rule fires. Without these the facet check could
    /// be off by one (or reject everything) and (3c'') would still pass. [OPUS-5]
    #[test]
    fn body_equal_derived_integer_facet_boundaries_are_valid_and_decided() {
        for (l, r) in [
            (("1", "positiveInteger"), ("1", "integer")),
            (("0", "nonNegativeInteger"), ("0", "integer")),
            (("0", "nonPositiveInteger"), ("0", "integer")),
            (("-1", "negativeInteger"), ("-1", "integer")),
            (("127", "byte"), ("127", "integer")),
            (("-128", "byte"), ("-128.0", "decimal")),
            (("255", "unsignedByte"), ("255", "integer")),
            (("0", "unsignedLong"), ("-0.0", "double")),
            (("18446744073709551615", "unsignedLong"), ("18446744073709551615", "integer")),
            // `xsd:integer` itself is unfaceted: an arbitrarily large value stays valid.
            (
                ("170141183460469231731687303715884105728", "integer"),
                ("+170141183460469231731687303715884105728", "integer"),
            ),
        ] {
            let d = distinct_ground_equal_doc(l, r);
            d.validate().unwrap_or_else(|e| {
                panic!("{:?} is inside its datatype's value space and must validate: {}", l, e)
            });
            let mut dict = Dict::new();
            let closure = d.closure(&mut dict).expect("closure succeeds");
            assert!(
                has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
                "{:?} = {:?} is a valid value-EQUAL pair, so the rule must fire",
                l,
                r
            );
        }
    }

    /// (3c') Well-formed `xsd:integer`/`xsd:decimal` values BEYOND the shared tower's
    /// `i128` mantissa are still DECIDED — `as_numeric` classifies them out, so the
    /// exact plain-decimal fallback compares the lexicals by string arithmetic. Two
    /// lexical forms of the same >i128 value (leading `+`, padding zeros, decimal
    /// spelling of an integer) are value-EQUAL, so the rule fires. [OPUS-5]
    #[test]
    fn body_equal_out_of_tower_value_equal_numerics_are_decided_and_the_rule_fires() {
        // i128::MAX + 1 — one past the exact tower, so `as_numeric` returns None.
        const HUGE: &str = "170141183460469231731687303715884105728";
        let signed = format!("+{}", HUGE);
        let padded = format!("000{}", HUGE);
        let as_decimal = format!("{}.00", HUGE);
        let frac = format!("{}.5", HUGE);
        let frac_padded = format!("+{}.50", HUGE);
        for (l, r) in [
            // The reviewer's case: the same >i128 integer with and without a `+`.
            ((HUGE, "integer"), (signed.as_str(), "integer")),
            // Leading zeros are insignificant in the value space.
            ((HUGE, "integer"), (padded.as_str(), "integer")),
            // Cross-tier: the decimal spelling of the same huge integer.
            ((HUGE, "integer"), (as_decimal.as_str(), "decimal")),
            // Both sides out of tower, in the decimal tier.
            ((frac.as_str(), "decimal"), (frac_padded.as_str(), "decimal")),
        ] {
            let d = distinct_ground_equal_doc(l, r);
            d.validate().unwrap_or_else(|e| {
                panic!("value-equal out-of-tower numerics {:?} = {:?} must validate: {}", l, r, e)
            });
            let mut dict = Dict::new();
            let closure = d.closure(&mut dict).expect("closure succeeds");
            assert!(
                has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
                "{:?} = {:?} is value-space TRUE beyond i128, so the rule must fire",
                l,
                r
            );
        }
    }

    /// (3c'') The mirror: UNEQUAL well-formed values beyond the tower make the rule
    /// VACUOUS (it validates and derives nothing) rather than failing closed — including
    /// a pair that differs only past the i128 boundary, which a lossy `f64` fallback
    /// would wrongly report equal. [OPUS-5]
    #[test]
    fn body_equal_out_of_tower_value_unequal_numerics_make_the_rule_vacuous() {
        const HUGE: &str = "170141183460469231731687303715884105728";
        // Differs from HUGE by ONE — far below an f64 ulp at this magnitude, so a lossy
        // float fallback would wrongly report the pair equal. String arithmetic does not.
        const HUGE_PLUS_ONE: &str = "170141183460469231731687303715884105729";
        let negated = format!("-{}", HUGE);
        let frac5 = format!("{}.5", HUGE);
        let frac6 = format!("{}.6", HUGE);
        for (l, r) in [
            ((HUGE, "integer"), (HUGE_PLUS_ONE, "integer")),
            ((HUGE, "integer"), (negated.as_str(), "integer")),
            ((frac5.as_str(), "decimal"), (frac6.as_str(), "decimal")),
        ] {
            let d = distinct_ground_equal_doc(l, r);
            d.validate().unwrap_or_else(|e| {
                panic!("an unsatisfiable-but-safe out-of-tower rule is valid: {}", e)
            });
            let mut dict = Dict::new();
            let closure = d.closure(&mut dict).expect("closure succeeds");
            assert!(
                !has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
                "{:?} = {:?} is value-space FALSE beyond i128, so the rule derives NOTHING",
                l,
                r
            );
        }
    }

    /// (3d) A `NaN` operand is a comparator TYPE ERROR (`cmp_relational` returns `None`),
    /// not a verdict — so it fails closed rather than being silently answered `false`.
    /// Two syntactically IDENTICAL `NaN` terms are still trivially true by RIF-Core
    /// ground identity (they never reach the value-space path).
    #[test]
    fn body_equal_nan_operand_fails_closed_but_identity_still_holds() {
        let d = distinct_ground_equal_doc(("NaN", "double"), ("1", "double"));
        assert!(
            matches!(d.validate(), Err(RifError::DistinctGroundEqual { .. })),
            "a NaN operand is a type error for cmp_relational -> fail closed"
        );
        let identical = distinct_ground_equal_doc(("NaN", "double"), ("NaN", "double"));
        identical.validate().expect("identical terms are ground-identical (reflexivity)");
    }

    /// (3e) Non-literal operands (IRIs, `List`s) are never numeric, so they keep the
    /// pre-existing fail-closed behaviour.
    #[test]
    fn body_equal_iri_and_list_grounds_still_fail_closed() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: iri("http://ex/s"), class: iri("http://ex/C") }],
            vec![Atom::Equal { left: iri("http://ex/a"), right: iri("http://ex/b") }],
        ));
        assert!(
            matches!(d.validate(), Err(RifError::DistinctGroundEqual { .. })),
            "distinct IRI constants are not numerically decidable"
        );
        let mut d2 = Document::new();
        d2.push(Rule::implies(
            vec![Atom::Member { obj: iri("http://ex/s"), class: iri("http://ex/C") }],
            vec![Atom::Equal {
                left: Term::List(vec![Term::int(1)]),
                right: Term::int(1),
            }],
        ));
        assert!(
            matches!(d2.validate(), Err(RifError::DistinctGroundEqual { .. })),
            "a List operand is not a numeric literal"
        );
    }

    /// (3f) The value-space check is re-run on terms CREATED by substitution: `?x = 1 ,
    /// ?y = "1.0"^^xsd:decimal , ?x = ?y` reduces the last atom to `1 = 1.0`, which is
    /// value-space TRUE — so the rule fires (the mirror of the `<a> = <b>` fail-closed
    /// case below).
    #[test]
    fn body_equal_subst_created_numeric_ground_is_decided() {
        let dec = |lex: &str| Term::Lit {
            lex: lex.to_string(),
            datatype: format!("{}decimal", XSD),
        };
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: iri("http://ex/s"), class: iri("http://ex/C") }],
            vec![
                Atom::Equal { left: var("x"), right: Term::int(1) },
                Atom::Equal { left: var("y"), right: dec("1.0") },
                Atom::Equal { left: var("x"), right: var("y") },
            ],
        ));
        d.validate().expect("substitution-created 1 = 1.0 is value-space TRUE");
        let mut dict = Dict::new();
        let closure = d.closure(&mut dict).expect("closure succeeds");
        assert!(
            has(&dict, &closure, "http://ex/s", RDF_TYPE, "http://ex/C"),
            "the substitution-created numeric equality holds, so the fact is derived"
        );
    }

    // ---- [OPUS-4.8] sq-26vwp — variable / mixed Equal via compile-time substitution ----

    /// V2 (under-derivation gone): `?x = ?y` is UNIFIED at compile time, so the rule
    /// fires exactly when the two variables bind the SAME node — with NO `owl:sameAs`
    /// assertion needed (RIF reflexivity `t = t` is honoured). Exercises `closure`.
    #[test]
    fn body_equal_var_unification_same_node_fires() {
        // Rule: ?x # SelfManaged :- ?x manager ?y , ?x = ?y
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/alice"),
            pred: iri("http://ex/manager"),
            val: iri("http://ex/alice"),
        }));
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/bob"),
            pred: iri("http://ex/manager"),
            val: iri("http://ex/carol"),
        }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/SelfManaged") }],
            vec![
                Atom::Frame { obj: var("x"), pred: iri("http://ex/manager"), val: var("y") },
                Atom::Equal { left: var("x"), right: var("y") },
            ],
        ));
        doc.validate().expect("?x=?y unifies; rule is valid");
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure succeeds");
        assert!(
            has(&dict, &closure, "http://ex/alice", RDF_TYPE, "http://ex/SelfManaged"),
            "alice manages herself (same node) -> SelfManaged, no owl:sameAs needed (V2 fixed)"
        );
        assert!(
            !has(&dict, &closure, "http://ex/bob", RDF_TYPE, "http://ex/SelfManaged"),
            "bob's manager is carol (distinct node) -> NOT SelfManaged"
        );
    }

    /// V1 (over-derivation gone): an asserted `owl:sameAs` DATA triple between
    /// DISTINCT nodes must NOT satisfy `?x = ?y`. Equality is compile-time identity,
    /// not the `owl:sameAs` vocabulary. The same rule DOES fire on a genuine
    /// same-node pair, proving it is not vacuously false.
    #[test]
    fn body_equal_var_no_sameas_over_derivation() {
        let owl_same = "http://www.w3.org/2002/07/owl#sameAs";
        // Rule: ?x # SelfRel :- ?x rel ?y , ?x = ?y
        let mut doc = Document::new();
        // a rel b  (a and b are DISTINCT nodes)
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri("http://ex/rel"),
            val: iri("http://ex/b"),
        }));
        // a owl:sameAs b  (a vocabulary DATA triple — must NOT license RIF equality)
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri(owl_same),
            val: iri("http://ex/b"),
        }));
        // c rel c  (same node — SHOULD fire, so the rule is non-vacuous)
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/c"),
            pred: iri("http://ex/rel"),
            val: iri("http://ex/c"),
        }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/SelfRel") }],
            vec![
                Atom::Frame { obj: var("x"), pred: iri("http://ex/rel"), val: var("y") },
                Atom::Equal { left: var("x"), right: var("y") },
            ],
        ));
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure succeeds");
        assert!(
            !has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/SelfRel"),
            "a rel b with an asserted a owl:sameAs b must NOT derive a # SelfRel (V1 fixed)"
        );
        assert!(
            has(&dict, &closure, "http://ex/c", RDF_TYPE, "http://ex/SelfRel"),
            "c rel c (same node) DOES derive c # SelfRel -> the rule is non-vacuous"
        );
    }

    /// `?v = <target>` substitutes the ground term for the variable throughout the
    /// rule; the rule then fires only where the substituted constant matches.
    #[test]
    fn body_equal_var_ground_substitution_end_to_end() {
        // Rule: ?x # Special :- ?x p ?v , ?v = <target>
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/a"),
            pred: iri("http://ex/p"),
            val: iri("http://ex/target"),
        }));
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/b"),
            pred: iri("http://ex/p"),
            val: iri("http://ex/other"),
        }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/Special") }],
            vec![
                Atom::Frame { obj: var("x"), pred: iri("http://ex/p"), val: var("v") },
                Atom::Equal { left: var("v"), right: iri("http://ex/target") },
            ],
        ));
        doc.validate().expect("?v=<target> substitutes; valid");
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure succeeds");
        assert!(
            has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/Special"),
            "a p target -> Special (substitution ?v:=target)"
        );
        assert!(
            !has(&dict, &closure, "http://ex/b", RDF_TYPE, "http://ex/Special"),
            "b p other -> NOT Special"
        );
    }

    /// A head variable bound SOLELY by `?x = <a>` is bound-by-substitution (contrast
    /// the `?x = ?x` sole-binder case, which is `UnboundHeadVar`): the rule collapses
    /// to the unconditional fact `<a> # C`, and NO `owl:sameAs` is ever emitted.
    #[test]
    fn body_equal_var_ground_binds_head_var() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: var("x"), class: iri("http://ex/C") }],
            vec![Atom::Equal { left: var("x"), right: iri("http://ex/a") }],
        ));
        assert_eq!(d.validate(), Ok(()), "?x=<a> binds ?x by substitution -> valid");
        // to_n3 collapses the rule to a FACT (empty resolved body) and emits no sameAs.
        let src = d.to_n3_source().unwrap();
        assert!(!src.contains("=>"), "rule with only ?x=<a> collapses to a fact (no `=>`)");
        assert!(!src.contains("sameAs"), "no owl:sameAs is ever emitted");
        let mut dict = Dict::new();
        let closure = d.closure(&mut dict).expect("closure succeeds");
        assert!(
            has(&dict, &closure, "http://ex/a", RDF_TYPE, "http://ex/C"),
            "<a> # C is derived unconditionally (?x:=<a>)"
        );
    }

    /// Chained equalities `?x = ?y , ?y = <target>` collapse both variables to the
    /// ground term via fixpoint substitution.
    #[test]
    fn body_equal_chained_collapse_to_ground() {
        // Rule: ?a # Matched :- ?a p ?x , ?x = ?y , ?y = <target>
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/s"),
            pred: iri("http://ex/p"),
            val: iri("http://ex/target"),
        }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("a"), class: iri("http://ex/Matched") }],
            vec![
                Atom::Frame { obj: var("a"), pred: iri("http://ex/p"), val: var("x") },
                Atom::Equal { left: var("x"), right: var("y") },
                Atom::Equal { left: var("y"), right: iri("http://ex/target") },
            ],
        ));
        doc.validate().expect("chained equalities resolve; valid");
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure succeeds");
        assert!(
            has(&dict, &closure, "http://ex/s", RDF_TYPE, "http://ex/Matched"),
            "s p target -> Matched (chained ?x=?y, ?y=target both become target)"
        );
    }

    /// A substitution can CREATE a ground identity: `?x = <a> , ?y = <a> , ?x = ?y`
    /// makes the last atom reduce to `<a> = <a>`, eliminated (re-run to fixpoint).
    #[test]
    fn body_equal_subst_creates_ground_identity() {
        // Rule: ?z # C :- ?z p ?x , ?x = <a> , ?y = <a> , ?x = ?y
        let mut doc = Document::new();
        doc.push(Rule::fact(Atom::Frame {
            obj: iri("http://ex/s"),
            pred: iri("http://ex/p"),
            val: iri("http://ex/a"),
        }));
        doc.push(Rule::implies(
            vec![Atom::Member { obj: var("z"), class: iri("http://ex/C") }],
            vec![
                Atom::Frame { obj: var("z"), pred: iri("http://ex/p"), val: var("x") },
                Atom::Equal { left: var("x"), right: iri("http://ex/a") },
                Atom::Equal { left: var("y"), right: iri("http://ex/a") },
                Atom::Equal { left: var("x"), right: var("y") },
            ],
        ));
        doc.validate().expect("substitution-created ground identity eliminated; valid");
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure succeeds");
        assert!(
            has(&dict, &closure, "http://ex/s", RDF_TYPE, "http://ex/C"),
            "s p a -> C (x=a, y=a make x=y collapse to a=a, eliminated)"
        );
    }

    /// A substitution can CREATE a distinct-ground equality: `?x = <a> , ?y = <b> ,
    /// ?x = ?y` makes the last atom reduce to `<a> = <b>` -> fail-closed
    /// `DistinctGroundEqual` (IRIs are not numerically decidable — sq-anyad),
    /// re-derived after substitution.
    #[test]
    fn body_equal_subst_creates_distinct_ground_fail_closed() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: iri("http://ex/s"), class: iri("http://ex/C") }],
            vec![
                Atom::Equal { left: var("x"), right: iri("http://ex/a") },
                Atom::Equal { left: var("y"), right: iri("http://ex/b") },
                Atom::Equal { left: var("x"), right: var("y") },
            ],
        ));
        assert!(
            matches!(d.validate(), Err(RifError::DistinctGroundEqual { .. })),
            "substitution-created distinct ground (a=b) must fail closed"
        );
        let mut dict = Dict::new();
        assert!(
            d.closure(&mut dict).is_err(),
            "closure refuses a substitution-created distinct-ground equality"
        );
    }

    /// Occurs-check: `?x = (?x)` (a variable equal to a list containing itself) is
    /// cyclic and is rejected fail-closed rather than looped.
    #[test]
    fn body_equal_occurs_check_fails_closed() {
        let mut d = Document::new();
        d.push(Rule::implies(
            vec![Atom::Member { obj: iri("http://ex/s"), class: iri("http://ex/C") }],
            vec![Atom::Equal { left: var("x"), right: Term::List(vec![var("x")]) }],
        ));
        assert!(d.validate().is_err(), "cyclic ?x = (?x) must fail closed (occurs-check)");
    }

    #[test]
    fn rif_error_displays() {
        let e = RifError::UnboundHeadVar { var: "y".to_string() };
        assert!(e.to_string().contains("range-restricted"));
        let e = RifError::Nonmonotonic { what: "Naf".to_string() };
        assert!(e.to_string().contains("monotone"));
        let e = RifError::BadBuiltinArity { op: Builtin::NumericAdd, got: 1, want: 3 };
        assert!(e.to_string().contains("argument"));
        let e = RifError::BuiltinInHead { op: Builtin::NumericLessThan };
        assert!(e.to_string().contains("body-only"));
        let e = RifError::UnboundBuiltinInput { var: "n".to_string() };
        assert!(e.to_string().contains("range-restricted"));
        // [SONNET-4.6] sq-pbz04.5.4 — new error variants
        let e = RifError::EqualInConclusion;
        assert!(
            e.to_string().contains("conclusion"),
            "EqualInConclusion display mentions conclusion"
        );
        // [SONNET-4.6] sq-anyad — the message now names the NON-numeric half as the
        // remaining deferral (the numeric half is implemented).
        let e = RifError::DistinctGroundEqual {
            left: "true^^boolean".into(),
            right: "1^^boolean".into(),
        };
        assert!(
            e.to_string().contains("sq-anyad"),
            "DistinctGroundEqual display references the deferral bead"
        );
        assert!(
            e.to_string().contains("pred:boolean-equal"),
            "DistinctGroundEqual display names the still-deferred non-numeric half"
        );
    }

    #[test]
    fn to_n3_source_emits_facts_and_rules() {
        let mut d = Document::new();
        d.push(Rule::fact(Atom::Member { obj: iri("http://ex/a"), class: iri("http://ex/C") }));
        d.push(Rule::implies(
            vec![Atom::Subclass { sub: var("c"), sup: iri("http://ex/Top") }],
            vec![Atom::Subclass { sub: var("c"), sup: iri("http://ex/Mid") }],
        ));
        let src = d.to_n3_source().unwrap();
        assert!(src.contains("=>"), "the rule lowers to an N3 implication");
        assert!(src.contains("rdf-syntax-ns#type"), "membership lowers to rdf:type");
        assert!(src.contains("subClassOf"), "subclass lowers to rdfs:subClassOf");
    }

    #[test]
    fn unimplemented_lists_naf_exclusion() {
        assert!(
            UNIMPLEMENTED.iter().any(|s| s.contains("Naf") && s.contains("monotone")),
            "the documented out-of-scope list names the NAF exclusion"
        );
    }
}
