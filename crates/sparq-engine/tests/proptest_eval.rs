//! [FABLE-5] sq-3dyje.3 — Property-based tests for sparq-engine evaluation:
//! engine ≡ independent naive reference on random graphs × random queries,
//! join-order permutation invariance, and ORDER BY laws over the real `Value` path.
//!
//! Design record: research/testing-strategy-assessment-2026-07.md §7.1 (sparq-engine row);
//! comparison model: research/differential-testing-value-level.md §2–§3.
//!
//! # The three property families
//!
//! 1. **`eval ≡ naive reference` (`family1_*`).** A random small graph and a random
//!    query (BGP / OPTIONAL / UNION — including one level of Opt/Union NESTING
//!    under a top-level Opt/Union, issue #2442 — / FILTER / BIND / DISTINCT /
//!    subset projection / ORDER BY over 1–2 keys / LIMIT / OFFSET) are generated;
//!    the engine's solution multiset must equal an in-test naive nested-loop
//!    evaluator's under MULTISET semantics (§2.1 of the comparison-model record).
//!    ORDER BY output is additionally checked for sortedness (see family 3); any
//!    LIMIT/OFFSET slice without a total order is checked as sub-multiset +
//!    cardinality (§2.2: which key-tied rows survive a slice is
//!    implementation-defined, but HOW MANY survive is not).
//! 2. **Join-order invariance (`family2_*`).** Permuting the triple patterns of a
//!    BGP never changes the result multiset — the planner (greedy GOO, or DPccp
//!    under `dp-planner`) reorders for performance only.
//! 3. **ORDER BY laws (`family3_*` + the sortedness check inside family 1).**
//!    Over tie-free integer sort keys the engine's ASC sequence equals the oracle's
//!    exact sort, DESC is its exact reverse, LIMIT k is the exact k-prefix, and
//!    OFFSET j (with or without LIMIT) is the exact window. Over arbitrary
//!    mixed-kind keys the engine sequence must respect the engine's DOCUMENTED
//!    total order (`sparq_substrate::compare::compare_terms`, the Kani-proved
//!    sq-wjl8i order): unbound < blank < IRI < literal; literals KIND-FIRST
//!    (numeric < boolean < string < langString); IRIs / xsd:strings by codepoint;
//!    numerics by value with NaN totalised FIRST (before -INF, equal to itself);
//!    booleans false < true; langStrings lex-first (tag ties engine-defined,
//!    unconstrained here — sq-ilweo promoted these arms from "unconstrained").
//!    Only bnode-label pairs remain deliberately unconstrained. For a two-key
//!    ORDER BY (#2442) the primary key is checked over the whole sequence and the
//!    secondary key within each maximal run of identical-RENDERED primary cells —
//!    such a run is provably a subsequence of one primary tie group, so the check
//!    is sound without modelling value-equal cross-lexical ties.
//!
//! # Oracle independence
//!
//! The reference evaluator shares NO code with the engine: it works on a test-side
//! term model (`T`) and a test-side query AST, evaluates by substitution-based
//! nested loops over a plain `Vec<[T; 3]>`, and compares by rendered N-Triples
//! strings. The engine is driven end-to-end through `sparq_engine::query` on the
//! SPARQL text rendered from the same AST — the engine's parser, planner, and
//! executor are all inside the tested boundary; the oracle touches none of them
//! (the §2.3 "independence trap" of the comparison-model record).
//!
//! # Plan paths
//!
//! With default features the engine path is the greedy GOO planner + scalar exec.
//! Compiled with `--features dp-planner,vectorized`, DPccp plans by DEFAULT
//! (tests/dp_default.rs) and the vectorized kernels engage — every property then
//! ALSO cross-checks `without_dp_planner` (greedy) against the same oracle, so one
//! run validates both plan paths case-by-case.
//!
//! # Pinned engine semantics the oracle encodes (probe-verified in this tree)
//!
//! Each rule below was pinned by running the real engine before writing the oracle:
//!
//! * literals keep their lexical form end-to-end (`"1.50"^^xsd:decimal` stays "1.50");
//!   blank-node labels round-trip through load+query.
//! * `=` on literals: numeric×numeric / string×string / boolean×boolean compare by
//!   value; langString = anything-not-langString → `false`; any other cross-class
//!   literal pair (string×numeric, boolean×numeric, boolean×string) → type error.
//!   `=` across term kinds (IRI / bnode / literal) → `false`. Unbound operand → error.
//! * `<` `>` against an integer constant: numeric operand compares by value; any
//!   non-numeric operand (IRI, bnode, string, boolean, langString) → type error.
//! * `<` `>` var against var (issue #2443): the strict comparator
//!   (`value_compare_strict`) decides only same-family pairs — numeric×numeric by
//!   value (a NaN operand → type error, as above), xsd:string×xsd:string by
//!   codepoint, boolean×boolean with false < true, and SAME-TAG langString pairs
//!   by lexical (the lenient engine extension: `FILTER("a"@en < "b"@en)` is TRUE);
//!   a cross-tag langString pair and every cross-kind pair (IRI or bnode operands
//!   included) → type error.
//! * three-valued logic is exactly SPARQL's: `err||true=true`, `err||false=err`,
//!   `err&&false=false`, `!err=err`; a FILTER error drops the row.
//! * `BIND` arithmetic: int+int → canonical integer; decimal+int keeps the operand's
//!   scale ("1.50"+1 → "2.50"); a type-error BIND leaves the variable UNBOUND and
//!   keeps the row. Computed booleans render as `"true|false"^^xsd:boolean`.
//! * computed xsd:double lexicals (sq-ilweo): a double+int BIND promotes to double
//!   and renders via the arithmetic convention (`Num::lexical` → `fmt_xsd_double`):
//!   `NaN`/`INF`/`-INF` keep their XSD spellings; an INTEGRAL value with |v| < 1e15
//!   prints as a PLAIN integer ("101", and -0.0 → "0"); anything else prints as the
//!   shortest-round-trip mantissa-E-exponent form with ≥ 1 fraction digit ("2.5E0",
//!   "2.0E-1"). The oracle re-derives this INDEPENDENTLY via the `ryu` shortest
//!   formatter (`fmt_double_oracle`), not the engine's `format!("{:E}")` path.
//! * xsd:double NaN (sq-ilweo): `=` on NaN is sameTerm-true for the identical
//!   `"NaN"^^xsd:double` term (the open-world identical-terms fast path) and FALSE
//!   against any other numeric (op:numeric-equal is undecided → known-different);
//!   `<` `>` against a numeric constant is a TYPE ERROR (the stored-NaN numeric
//!   cache reads back as a miss, so NaN takes the strict path, where num_compare is
//!   undecided); ORDER BY totalises NaN FIRST — before -INF, equal to itself.
//! * ORDER BY: unbound < blank < IRI < literal; literals KIND-FIRST per the
//!   substrate `LiteralKind` rank (numeric < boolean < string < langString);
//!   IRIs / xsd:strings by codepoint; numerics by value across int/decimal/double
//!   (NaN first); booleans false < true; langStrings by lexical, tag ties
//!   engine-defined.
//! * OFFSET j (with optional LIMIT k) over an oracle multiset of N rows returns
//!   exactly `min(k, max(N - j, 0))` rows, each drawn from the oracle multiset
//!   (§2.2 cardinality-only comparison — WHICH rows survive is only pinned under a
//!   tie-free total order, checked in family 3).
//!
//! # Documented generator narrowings (each carries a reason + a follow-up bead)
//!
//! * (RESOLVED, generation re-widened — sq-qeltv [FABLE-5]) `is*()` used to be
//!   restricted to certainly-bound vars and the STRLEN-BIND form used to mask blank
//!   nodes out of the graph, because the engine leniently returned `false` for
//!   `isIRI(?unbound)` and an internal label for `STR(bnode)`. Both are TYPE ERRORS
//!   per SPARQL 1.1 §17.2/§17.4.2 (differential: Oxigraph errors on both) and the
//!   engine now conforms, so `is*()` ranges over ALL in-scope vars (including
//!   possibly-unbound OPTIONAL/UNION/BIND vars) and `STR` sees bnodes.
//! * (RESOLVED, generation re-widened — sq-ilweo [FABLE-5]) `?v + k` used to mask
//!   xsd:double literals out of the generated graph (and strip pattern-const
//!   doubles) because rendering a computed double would have required replicating
//!   the engine's float formatter inside the oracle. The oracle now derives the
//!   lexical INDEPENDENTLY (`fmt_double_oracle`, built on the `ryu` shortest
//!   round-trip formatter — a different implementation than the std `{:E}` path the
//!   engine uses), with the engine's computed-double rules pinned first by
//!   `oracle_known_answer_computed_double`. Doubles now flow into every generated
//!   graph unconditionally.
//! * (RESOLVED, generation re-widened — sq-ilweo [FABLE-5]) xsd:double NaN used to
//!   be excluded because `<` is not a total order under NaN. The engine's
//!   documented NaN totalisation (substrate `compare_terms`, Kani-proved under
//!   sq-wjl8i, comparator-level coverage in
//!   sparq-substrate/tests/proptest_order_numeric.rs) is NaN-FIRST, so the ORDER BY
//!   law now encodes it and NaN is generated in graphs, pattern constants, and
//!   FILTER constants (value semantics pinned by `oracle_known_answer_nan`).
//!
//! # Non-vacuity (mutation-verified, see PR)
//!
//! Verified by temporarily injecting engine bugs into `src/exec.rs` and observing
//! the properties go red with shrunk counterexamples (then reverting; the committed
//! src tree is byte-identical to main):
//! * FILTER-drop (`apply_filter_scalar`: `keep` overridden to all-`true`) → family 1
//!   red, shrunk to a 1-pattern query with `FILTER(!isIRI(?v0))`.
//! * join-dup (`bind_join`: `sjoin::bind_combine` emitted twice) → families 1 AND 2
//!   red (every joined row duplicated vs the oracle multiset).
//! * DISTINCT no-op (`distinct_bindings` emptied) → family 1 red on a
//!   `SELECT DISTINCT` query whose oracle multiset deduped.
//!
//! Additionally `oracle_known_answer_*` pin the oracle itself against hand-computed
//! rows (a degenerate oracle cannot silently agree with the engine), and
//! `generator_diversity_floor` asserts the generated corpus actually contains
//! nonempty results, duplicate rows, unbound cells, and every query shape.

use proptest::prelude::*;
use sparq_core::Graph;
use sparq_engine::query;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

// ═══════════════════════════════════════════════════════════════════════════════
// Test-side term model (independent of the engine's Value / dict code)
// ═══════════════════════════════════════════════════════════════════════════════

/// A test-side RDF term. Carries exactly the information needed to (a) emit the
/// N-Triples data / SPARQL query text and (b) evaluate the naive reference.
#[derive(Clone, Debug)]
enum T {
    Iri(String),
    BNode(String),
    /// simple literal ≡ xsd:string (RDF 1.1); rendered without a datatype suffix,
    /// which is also how the engine's `Term::to_string()` renders xsd:string.
    Str(String),
    /// language-tagged string: (lexical, lowercase tag).
    Lang(String, String),
    Int(i64),
    /// xsd:decimal as (mantissa, scale), scale ≥ 1; lexical is the canonical
    /// scaled rendering (`render_dec`).
    Dec(i128, u32),
    /// xsd:double as (lexical form, value): data-sourced from the fixed pool, or
    /// computed by a `PlusK` BIND (lexical via `fmt_double_oracle` — sq-ilweo).
    Dbl(String, f64),
    Bool(bool),
}

/// TERM identity, not value identity — the semantics of BGP matching and of the
/// oracle's join compatibility. `Dbl` compares by LEXICAL: `"0.0E0"` and `"-0.0E0"`
/// are DISTINCT terms even though their values compare equal, and the `"NaN"` term
/// EQUALS itself even though `NaN != NaN` as an f64 (a derived PartialEq would make
/// a NaN data triple fail to match its own pattern constant). Mirrors the engine's
/// canonicalising dictionary, which interns literals by (lexical, datatype).
impl PartialEq for T {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (T::Iri(a), T::Iri(b)) | (T::BNode(a), T::BNode(b)) | (T::Str(a), T::Str(b)) => a == b,
            (T::Lang(a, ta), T::Lang(b, tb)) => a == b && ta == tb,
            (T::Int(a), T::Int(b)) => a == b,
            (T::Dec(ma, sa), T::Dec(mb, sb)) => ma == mb && sa == sb,
            (T::Dbl(a, _), T::Dbl(b, _)) => a == b,
            (T::Bool(a), T::Bool(b)) => a == b,
            _ => false,
        }
    }
}

fn render_dec(m: i128, scale: u32) -> String {
    let neg = m < 0;
    let abs = m.unsigned_abs();
    let pow = 10u128.pow(scale);
    let int_part = abs / pow;
    let frac = abs % pow;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&int_part.to_string());
    s.push('.');
    let frac_str = frac.to_string();
    for _ in frac_str.len()..(scale as usize) {
        s.push('0');
    }
    s.push_str(&frac_str);
    s
}

/// INDEPENDENT re-derivation of the engine's computed-xsd:double lexical rules
/// (`sparq_substrate`'s `fmt_xsd_double`, pinned end-to-end by
/// `oracle_known_answer_computed_double`): specials keep their XSD spellings; an
/// integral |v| < 1e15 prints as a plain integer; everything else prints as the
/// shortest-round-trip mantissa-E-exponent form with ≥ 1 fraction digit.
///
/// Independence (the §2.3 trap, sq-ilweo): the engine's non-integral arm formats
/// with std's `format!("{v:E}")` (Grisu-style shortest with exact fallback); this
/// oracle instead takes the shortest-round-trip DIGITS from the `ryu` crate — a
/// separate implementation of a provably UNIQUE output (the shortest correctly-
/// rounded decimal) — and normalises them to the same `m.mmE±e` convention.
fn fmt_double_oracle(v: f64) -> String {
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
        return format!("{}", v as i64); // note: -0.0 prints as "0"
    }
    let mut buf = ryu::Buffer::new();
    let s = buf.format_finite(v); // e.g. "2.5", "0.2", "1e16", "-101.5"
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let (digits_part, exp10): (&str, i32) = match s.split_once(['e', 'E']) {
        Some((d, e)) => (d, e.parse().expect("ryu exponent")),
        None => (s, 0),
    };
    let (int_d, frac_d) = digits_part.split_once('.').unwrap_or((digits_part, ""));
    let digits = format!("{}{}", int_d, frac_d);
    // exponent of the LEADING KEPT digit: position of the decimal point relative to
    // the first significant digit, plus any explicit exponent.
    let lead_zeros = digits.len() - digits.trim_start_matches('0').len();
    let exp = exp10 + int_d.len() as i32 - 1 - lead_zeros as i32;
    let kept = digits.trim_start_matches('0').trim_end_matches('0');
    let kept = if kept.is_empty() { "0" } else { kept };
    let mantissa = if kept.len() == 1 {
        format!("{}.0", kept)
    } else {
        format!("{}.{}", &kept[..1], &kept[1..])
    };
    format!("{}{}E{}", if neg { "-" } else { "" }, mantissa, exp)
}

impl T {
    /// N-Triples / SPARQL rendering. Matches the engine's `Term::to_string()` for
    /// every term this file can generate or compute (probe-verified).
    fn render(&self) -> String {
        match self {
            T::Iri(i) => format!("<{}>", i),
            T::BNode(b) => format!("_:{}", b),
            T::Str(s) => format!("\"{}\"", s),
            T::Lang(s, tag) => format!("\"{}\"@{}", s, tag),
            T::Int(n) => format!("\"{}\"^^<{}integer>", n, XSD),
            T::Dec(m, sc) => format!("\"{}\"^^<{}decimal>", render_dec(*m, *sc), XSD),
            T::Dbl(lex, _) => format!("\"{}\"^^<{}double>", lex, XSD),
            T::Bool(b) => format!("\"{}\"^^<{}boolean>", b, XSD),
        }
    }

    /// The SPARQL `STR()` of this term: IRI string or literal lexical form; a
    /// blank node is a TYPE ERROR (`None`) per SPARQL 1.1 §17.4.2.5 (sq-qeltv).
    fn str_value(&self) -> Option<String> {
        match self {
            T::Iri(i) => Some(i.clone()),
            T::BNode(_) => None,
            T::Str(s) | T::Lang(s, _) => Some(s.clone()),
            T::Int(n) => Some(n.to_string()),
            T::Dec(m, sc) => Some(render_dec(*m, *sc)),
            T::Dbl(lex, _) => Some(lex.clone()),
            T::Bool(b) => Some(b.to_string()),
        }
    }
}

/// Exact numeric value of a literal, if it is numeric.
#[derive(Clone, Debug)]
enum Num {
    I(i64),
    D(i128, u32),
    F(f64),
}

fn num_of(t: &T) -> Option<Num> {
    match t {
        T::Int(n) => Some(Num::I(*n)),
        T::Dec(m, s) => Some(Num::D(*m, *s)),
        T::Dbl(_, v) => Some(Num::F(*v)),
        _ => None,
    }
}

/// TOTAL value comparison on the numeric tower. int/decimal compare exactly in
/// i128; anything involving a double converts to f64 — exact for every value this
/// file generates (|int| ≤ 4000, decimal mantissa/10^scale is a correctly-rounded
/// quotient of small integers, doubles come from a fixed finite pool + NaN). NaN is
/// TOTALISED FIRST — before -INF, equal to itself — mirroring the engine's ORDER BY
/// order (substrate `compare_terms`, sq-wjl8i). The RELATIONAL operators guard NaN
/// BEFORE calling this (`eq_spec` / `cmp_int_spec`): only the total order positions
/// NaN.
fn num_cmp(a: &Num, b: &Num) -> std::cmp::Ordering {
    use Num::*;
    match (a, b) {
        (I(x), I(y)) => x.cmp(y),
        (D(mx, sx), D(my, sy)) => {
            let scale = (*sx).max(*sy);
            let ax = mx * 10i128.pow(scale - sx);
            let ay = my * 10i128.pow(scale - sy);
            ax.cmp(&ay)
        }
        (I(x), D(my, sy)) => num_cmp(&D(*x as i128, 0), &D(*my, *sy)),
        (D(mx, sx), I(y)) => num_cmp(&D(*mx, *sx), &D(*y as i128, 0)),
        _ => {
            let fx = num_f64(a);
            let fy = num_f64(b);
            match fx.partial_cmp(&fy) {
                Some(o) => o,
                None => match (fx.is_nan(), fy.is_nan()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                },
            }
        }
    }
}

fn num_f64(n: &Num) -> f64 {
    match n {
        Num::I(x) => *x as f64,
        Num::D(m, s) => (*m as f64) / 10f64.powi(*s as i32),
        Num::F(v) => *v,
    }
}

/// SPARQL `=` under the engine's probe-pinned model (see header table).
/// `Err(())` is a type error.
fn eq_spec(a: &T, b: &T) -> Result<bool, ()> {
    use T::*;
    match (a, b) {
        (Iri(x), Iri(y)) => Ok(x == y),
        (BNode(x), BNode(y)) => Ok(x == y),
        // different term kinds (or kind vs literal): definitely not the same term.
        (Iri(_) | BNode(_), _) | (_, Iri(_) | BNode(_)) => Ok(false),
        // literals from here on.
        (Lang(l1, t1), Lang(l2, t2)) => Ok(l1 == l2 && t1 == t2),
        (Lang(..), _) | (_, Lang(..)) => Ok(false),
        (Str(x), Str(y)) => Ok(x == y),
        (Bool(x), Bool(y)) => Ok(x == y),
        _ => match (num_of(a), num_of(b)) {
            (Some(x), Some(y)) => {
                if num_f64(&x).is_nan() || num_f64(&y).is_nan() {
                    // NaN `=` (probe-pinned, sq-ilweo): the IDENTICAL "NaN" term is
                    // equal via the engine's open-world sameTerm fast path; any
                    // OTHER numeric pairing is undecided by op:numeric-equal and
                    // therefore known-different (false, not a type error).
                    Ok(a == b)
                } else {
                    Ok(num_cmp(&x, &y) == std::cmp::Ordering::Equal)
                }
            }
            // cross-class among {numeric, string, boolean}: type error.
            _ => Err(()),
        },
    }
}

/// SPARQL `<` / `>` of a term against an integer constant: numeric-only, anything
/// else is a type error (probe-pinned). A NaN operand is ALSO a type error: the
/// stored-NaN numeric cache reads back as a miss, so the engine takes the strict
/// comparison path, where the NaN pair is undecided (sq-ilweo).
fn cmp_int_spec(t: &T, k: i64) -> Result<std::cmp::Ordering, ()> {
    let n = num_of(t).ok_or(())?;
    if num_f64(&n).is_nan() {
        return Err(());
    }
    Ok(num_cmp(&n, &Num::I(k)))
}

/// SPARQL `<` / `>` of a var against another var: the engine's STRICT relational
/// semantics (`value_compare_strict`, probe-pinned by
/// `oracle_known_answer_var_var_relational` — issue #2443). Only same-family pairs
/// decide: numeric×numeric by value (a NaN operand is undecided → type error,
/// exactly as in `cmp_int_spec`); xsd:string×xsd:string by codepoint;
/// boolean×boolean with false < true; langString×langString with the SAME tag by
/// lexical (the lenient engine extension — a cross-tag pair is a type error).
/// Every other pair — IRI or bnode operands, cross-kind literals — is a type error.
fn cmp_vv_spec(a: &T, b: &T) -> Result<std::cmp::Ordering, ()> {
    use T::*;
    match (a, b) {
        (Str(x), Str(y)) => Ok(x.cmp(y)),
        (Bool(x), Bool(y)) => Ok(x.cmp(y)),
        (Lang(x, tx), Lang(y, ty)) if tx == ty => Ok(x.cmp(y)),
        _ => match (num_of(a), num_of(b)) {
            (Some(x), Some(y)) => {
                if num_f64(&x).is_nan() || num_f64(&y).is_nan() {
                    Err(())
                } else {
                    Ok(num_cmp(&x, &y))
                }
            }
            _ => Err(()),
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Test-side query AST + SPARQL rendering
// ═══════════════════════════════════════════════════════════════════════════════

/// Variable ids: 0..=3 subject/object vars, 3..=4 predicate vars (3 is shared so
/// one variable can join across positions), 9 the BIND target.
type V = u8;
const BIND_VAR: V = 9;

fn vname(v: V) -> String {
    format!("?v{}", v)
}

#[derive(Clone, Debug, PartialEq)]
enum Pos {
    Var(V),
    Const(T),
}

#[derive(Clone, Debug, PartialEq)]
struct Tp {
    s: Pos,
    p: Pos,
    o: Pos,
}

impl Tp {
    fn render(&self) -> String {
        let r = |p: &Pos| match p {
            Pos::Var(v) => vname(*v),
            Pos::Const(t) => t.render(),
        };
        format!("{} {} {}", r(&self.s), r(&self.p), r(&self.o))
    }
    fn vars(&self, out: &mut Vec<V>) {
        for p in [&self.s, &self.p, &self.o] {
            if let Pos::Var(v) = p {
                if !out.contains(v) {
                    out.push(*v);
                }
            }
        }
    }
}

/// Recursive body: the generator produces at most ONE level of nesting below a
/// top-level Opt/Union (issue #2442, the sq-ilweo follow-up) — e.g.
/// `Opt(Union(BGP, BGP), Opt(BGP, BGP))` — mapping to the SPARQL algebra
/// LeftJoin/Union combinators evaluated bottom-up.
#[derive(Clone, Debug)]
enum Body {
    Bgp(Vec<Tp>),
    /// left OPTIONAL { right }  ≡  LeftJoin(left, right, true)
    Opt(Box<Body>, Box<Body>),
    /// { left } UNION { right }
    Union(Box<Body>, Box<Body>),
}

impl Body {
    fn render(&self) -> String {
        match self {
            Body::Bgp(tps) => tps.iter().map(Tp::render).collect::<Vec<_>>().join(" . "),
            Body::Opt(l, r) => {
                // A non-BGP left operand is wrapped in an explicit group so the
                // rendered text unambiguously parses as LeftJoin(left, right).
                let left = match l.as_ref() {
                    Body::Bgp(_) => l.render(),
                    nested => format!("{{ {} }}", nested.render()),
                };
                format!("{} OPTIONAL {{ {} }}", left, r.render())
            }
            Body::Union(a, b) => format!("{{ {} }} UNION {{ {} }}", a.render(), b.render()),
        }
    }
    /// All variables, sorted.
    fn vars(&self) -> Vec<V> {
        fn walk(b: &Body, out: &mut Vec<V>) {
            match b {
                Body::Bgp(tps) => {
                    for tp in tps {
                        tp.vars(out);
                    }
                }
                Body::Opt(l, r) | Body::Union(l, r) => {
                    walk(l, out);
                    walk(r, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(self, &mut out);
        out.sort_unstable();
        out
    }
}

#[derive(Clone, Debug)]
enum Bind {
    /// BIND((?v + k) AS ?v9)
    PlusK(V, i64),
    /// BIND((?v < k) AS ?v9)
    LtK(V, i64),
    /// BIND(STRLEN(STR(?v)) AS ?v9)
    StrLenOf(V),
}

impl Bind {
    fn render(&self) -> String {
        match self {
            Bind::PlusK(v, k) => format!("BIND(({} + {}) AS {})", vname(*v), k, vname(BIND_VAR)),
            Bind::LtK(v, k) => format!("BIND(({} < {}) AS {})", vname(*v), k, vname(BIND_VAR)),
            Bind::StrLenOf(v) => format!("BIND(STRLEN(STR({})) AS {})", vname(*v), vname(BIND_VAR)),
        }
    }
}

#[derive(Clone, Debug)]
enum Expr {
    EqVV(V, V),
    EqVC(V, T), // T is an IRI or a literal (bnodes are not legal in expressions)
    LtVK(V, i64),
    GtVK(V, i64),
    LtVV(V, V),
    GtVV(V, V),
    Bound(V),
    IsIri(V),
    IsBlank(V),
    IsLit(V),
    IsNum(V),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

impl Expr {
    fn render(&self) -> String {
        match self {
            Expr::EqVV(a, b) => format!("({} = {})", vname(*a), vname(*b)),
            Expr::EqVC(v, c) => format!("({} = {})", vname(*v), c.render()),
            Expr::LtVK(v, k) => format!("({} < {})", vname(*v), k),
            Expr::GtVK(v, k) => format!("({} > {})", vname(*v), k),
            Expr::LtVV(a, b) => format!("({} < {})", vname(*a), vname(*b)),
            Expr::GtVV(a, b) => format!("({} > {})", vname(*a), vname(*b)),
            Expr::Bound(v) => format!("BOUND({})", vname(*v)),
            Expr::IsIri(v) => format!("isIRI({})", vname(*v)),
            Expr::IsBlank(v) => format!("isBlank({})", vname(*v)),
            Expr::IsLit(v) => format!("isLiteral({})", vname(*v)),
            Expr::IsNum(v) => format!("isNumeric({})", vname(*v)),
            Expr::Not(e) => format!("(!{})", e.render()),
            Expr::And(a, b) => format!("({} && {})", a.render(), b.render()),
            Expr::Or(a, b) => format!("({} || {})", a.render(), b.render()),
        }
    }
}

#[derive(Clone, Debug)]
struct Q {
    body: Body,
    bind: Option<Bind>,
    filter: Option<Expr>,
    distinct: bool,
    /// Projected variables, sorted, nonempty, ⊆ in-scope vars.
    project: Vec<V>,
    /// ORDER BY keys, outermost first: (variable ∈ project, descending?).
    /// Empty = no ORDER BY; ≤ 2 keys generated, pairwise-distinct vars (#2442).
    order: Vec<(V, bool)>,
    limit: Option<usize>,
    /// OFFSET j — compared under the §2.2 cardinality-only model (sq-ilweo).
    offset: Option<usize>,
}

impl Q {
    fn render(&self) -> String {
        let mut s = String::from("SELECT ");
        if self.distinct {
            s.push_str("DISTINCT ");
        }
        for v in &self.project {
            s.push_str(&vname(*v));
            s.push(' ');
        }
        s.push_str("WHERE { ");
        s.push_str(&self.body.render());
        if let Some(b) = &self.bind {
            s.push(' ');
            s.push_str(&b.render());
        }
        if let Some(f) = &self.filter {
            s.push_str(" FILTER");
            s.push_str(&f.render());
        }
        s.push_str(" }");
        if !self.order.is_empty() {
            s.push_str(" ORDER BY");
            for (v, desc) in &self.order {
                if *desc {
                    s.push_str(&format!(" DESC({})", vname(*v)));
                } else {
                    s.push_str(&format!(" {}", vname(*v)));
                }
            }
        }
        if let Some(k) = self.limit {
            s.push_str(&format!(" LIMIT {}", k));
        }
        if let Some(j) = self.offset {
            s.push_str(&format!(" OFFSET {}", j));
        }
        s
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// The naive reference evaluator (nested loops; shares nothing with the engine)
// ═══════════════════════════════════════════════════════════════════════════════

/// A solution mapping over the small var universe.
type Row = [Option<T>; 10];

fn empty_row() -> Row {
    Default::default()
}

fn match_pos(p: &Pos, t: &T, row: &Row) -> Option<Option<(V, T)>> {
    match p {
        Pos::Const(c) => (c == t).then_some(None),
        Pos::Var(v) => match &row[*v as usize] {
            Some(bound) => (bound == t).then_some(None),
            None => Some(Some((*v, t.clone()))),
        },
    }
}

fn match_tp(tp: &Tp, triple: &[T; 3], row: &Row) -> Option<Row> {
    let mut ext = row.clone();
    for (pos, term) in [(&tp.s, &triple[0]), (&tp.p, &triple[1]), (&tp.o, &triple[2])] {
        match match_pos(pos, term, &ext) {
            None => return None,
            Some(None) => {}
            Some(Some((v, t))) => ext[v as usize] = Some(t),
        }
    }
    Some(ext)
}

fn eval_bgp(data: &[[T; 3]], tps: &[Tp]) -> Vec<Row> {
    let mut sols = vec![empty_row()];
    for tp in tps {
        let mut next = Vec::new();
        for sol in &sols {
            for triple in data {
                if let Some(ext) = match_tp(tp, triple, sol) {
                    next.push(ext);
                }
            }
        }
        sols = next;
    }
    sols
}

fn compatible_merge(l: &Row, r: &Row) -> Option<Row> {
    let mut out = l.clone();
    for i in 0..out.len() {
        match (&out[i], &r[i]) {
            (Some(a), Some(b)) if a != b => return None,
            (None, Some(b)) => out[i] = Some(b.clone()),
            _ => {}
        }
    }
    Some(out)
}

/// Bottom-up SPARQL algebra evaluation: Opt is LeftJoin(Ω_l, Ω_r, true) — each
/// left row joins every compatible right row, or survives alone if none is
/// compatible — and Union is multiset concatenation. Sub-bodies are evaluated
/// independently (compositional semantics), recursing one level for the nested
/// bodies of issue #2442.
fn eval_body(data: &[[T; 3]], body: &Body) -> Vec<Row> {
    match body {
        Body::Bgp(tps) => eval_bgp(data, tps),
        Body::Opt(l, r) => {
            let left = eval_body(data, l);
            let right = eval_body(data, r);
            let mut out = Vec::new();
            for lr in &left {
                let mut matched = false;
                for rr in &right {
                    if let Some(m) = compatible_merge(lr, rr) {
                        out.push(m);
                        matched = true;
                    }
                }
                if !matched {
                    out.push(lr.clone());
                }
            }
            out
        }
        Body::Union(a, b) => {
            let mut out = eval_body(data, a);
            out.extend(eval_body(data, b));
            out
        }
    }
}

/// BIND semantics: a type error leaves the variable unbound and KEEPS the row.
fn eval_bind(b: &Bind, row: &Row) -> Option<T> {
    match b {
        Bind::PlusK(v, k) => match row[*v as usize].as_ref()? {
            T::Int(n) => Some(T::Int(n + k)),
            T::Dec(m, s) => Some(T::Dec(m + (*k as i128) * 10i128.pow(*s), *s)),
            // double + int promotes to double (XPath promotion); the computed
            // lexical is re-derived independently of the engine (sq-ilweo).
            T::Dbl(_, v) => {
                let r = v + *k as f64;
                Some(T::Dbl(fmt_double_oracle(r), r))
            }
            _ => None,
        },
        Bind::LtK(v, k) => {
            let t = row[*v as usize].as_ref()?;
            cmp_int_spec(t, *k).ok().map(|o| T::Bool(o == std::cmp::Ordering::Less))
        }
        Bind::StrLenOf(v) => {
            let t = row[*v as usize].as_ref()?;
            t.str_value().map(|s| T::Int(s.chars().count() as i64))
        }
    }
}

/// Three-valued (Kleene) FILTER expression evaluation; `Err(())` is a type error.
fn eval_expr(e: &Expr, row: &Row) -> Result<bool, ()> {
    let get = |v: &V| row[*v as usize].as_ref().ok_or(());
    match e {
        Expr::EqVV(a, b) => eq_spec(get(a)?, get(b)?),
        Expr::EqVC(v, c) => eq_spec(get(v)?, c),
        Expr::LtVK(v, k) => Ok(cmp_int_spec(get(v)?, *k)? == std::cmp::Ordering::Less),
        Expr::GtVK(v, k) => Ok(cmp_int_spec(get(v)?, *k)? == std::cmp::Ordering::Greater),
        Expr::LtVV(a, b) => Ok(cmp_vv_spec(get(a)?, get(b)?)? == std::cmp::Ordering::Less),
        Expr::GtVV(a, b) => Ok(cmp_vv_spec(get(a)?, get(b)?)? == std::cmp::Ordering::Greater),
        Expr::Bound(v) => Ok(row[*v as usize].is_some()),
        // is* over an unbound var is a type error per SPARQL 17.2 (`get(v)?`);
        // the engine now conforms, so generation covers it (sq-qeltv widened).
        Expr::IsIri(v) => Ok(matches!(get(v)?, T::Iri(_))),
        Expr::IsBlank(v) => Ok(matches!(get(v)?, T::BNode(_))),
        Expr::IsLit(v) => Ok(!matches!(get(v)?, T::Iri(_) | T::BNode(_))),
        Expr::IsNum(v) => Ok(num_of(get(v)?).is_some()),
        Expr::Not(inner) => eval_expr(inner, row).map(|b| !b),
        Expr::And(a, b) => match (eval_expr(a, row), eval_expr(b, row)) {
            (Ok(false), _) | (_, Ok(false)) => Ok(false),
            (Ok(true), Ok(true)) => Ok(true),
            _ => Err(()),
        },
        Expr::Or(a, b) => match (eval_expr(a, row), eval_expr(b, row)) {
            (Ok(true), _) | (_, Ok(true)) => Ok(true),
            (Ok(false), Ok(false)) => Ok(false),
            _ => Err(()),
        },
    }
}

/// The full naive evaluation: body → BIND → FILTER → project → DISTINCT.
/// Returns the (unordered) solution multiset as rendered rows; ordering laws are
/// checked against the engine output separately.
fn oracle_eval(data: &[[T; 3]], q: &Q) -> Vec<Vec<Option<String>>> {
    let mut rows = eval_body(data, &q.body);
    if let Some(b) = &q.bind {
        for row in &mut rows {
            row[BIND_VAR as usize] = eval_bind(b, row);
        }
    }
    if let Some(f) = &q.filter {
        rows.retain(|row| eval_expr(f, row) == Ok(true));
    }
    let mut projected: Vec<Vec<Option<String>>> = rows
        .iter()
        .map(|row| {
            q.project
                .iter()
                .map(|v| row[*v as usize].as_ref().map(T::render))
                .collect()
        })
        .collect();
    if q.distinct {
        let mut seen = std::collections::HashSet::new();
        projected.retain(|r| seen.insert(row_key(r)));
    }
    projected
}

// ═══════════════════════════════════════════════════════════════════════════════
// Data → N-Triples, engine execution, and the comparison model
// ═══════════════════════════════════════════════════════════════════════════════

fn to_ntriples(data: &[[T; 3]]) -> String {
    let mut s = String::new();
    for t in data {
        s.push_str(&format!("{} {} {} .\n", t[0].render(), t[1].render(), t[2].render()));
    }
    s
}

/// An RDF graph is a SET of triples: the oracle deduplicates exactly like the store.
fn dedup_data(mut data: Vec<[T; 3]>) -> Vec<[T; 3]> {
    let mut seen = std::collections::HashSet::new();
    data.retain(|t| seen.insert(format!("{} {} {}", t[0].render(), t[1].render(), t[2].render())));
    data
}

const UNBOUND_KEY: &str = "\u{1}unbound\u{1}";

fn row_key(row: &[Option<String>]) -> String {
    row.iter()
        .map(|c| c.as_deref().unwrap_or(UNBOUND_KEY))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn multiset(rows: &[Vec<Option<String>>]) -> Vec<String> {
    let mut keys: Vec<String> = rows.iter().map(|r| row_key(r)).collect();
    keys.sort_unstable();
    keys
}

/// Run the engine end-to-end (parse → plan → execute) and render the rows.
fn engine_rows(g: &Graph, qtext: &str) -> Vec<Vec<Option<String>>> {
    let r = query(g, qtext).unwrap_or_else(|e| panic!("engine rejected generated query {:?}: {}", qtext, e));
    r.rows
        .iter()
        .map(|row| row.iter().map(|c| c.as_ref().map(|t| t.to_string())).collect())
        .collect()
}

/// Every plan path compiled into this binary, as (label, result-rows) pairs.
/// Default build: greedy GOO. With `dp-planner`: DPccp is the default path and the
/// greedy path is cross-checked via `without_dp_planner`. With `vectorized` the
/// vectorized kernels are engaged inside whichever planner runs.
fn engine_paths(g: &Graph, qtext: &str) -> Vec<(&'static str, Vec<Vec<Option<String>>>)> {
    #[allow(unused_mut)] // mut is only needed when dp-planner pushes the second path
    let mut out = vec![("default", engine_rows(g, qtext))];
    #[cfg(feature = "dp-planner")]
    out.push(("greedy(no-dp)", sparq_engine::without_dp_planner(|| engine_rows(g, qtext))));
    out
}

/// Parse a rendered engine cell back into the test-side model — only the term
/// shapes this file can generate or compute ever appear.
fn parse_rendered(s: &str) -> T {
    if let Some(inner) = s.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        return T::Iri(inner.to_string());
    }
    if let Some(label) = s.strip_prefix("_:") {
        return T::BNode(label.to_string());
    }
    assert!(s.starts_with('"'), "unexpected rendered term: {}", s);
    let close = s.rfind('"').expect("closing quote");
    let lex = &s[1..close];
    let suffix = &s[close + 1..];
    if suffix.is_empty() {
        return T::Str(lex.to_string());
    }
    if let Some(tag) = suffix.strip_prefix('@') {
        return T::Lang(lex.to_string(), tag.to_string());
    }
    let dt = suffix
        .strip_prefix("^^<")
        .and_then(|r| r.strip_suffix('>'))
        .unwrap_or_else(|| panic!("unexpected literal suffix in {}", s));
    match dt.strip_prefix(XSD) {
        Some("integer") => T::Int(lex.parse().expect("integer lexical")),
        Some("boolean") => T::Bool(lex == "true"),
        Some("decimal") => {
            let (int_part, frac) = lex.split_once('.').expect("decimal point");
            let neg = int_part.starts_with('-');
            let digits: i128 = format!("{}{}", int_part.trim_start_matches('-'), frac).parse().expect("decimal digits");
            T::Dec(if neg { -digits } else { digits }, frac.len() as u32)
        }
        Some("double") => {
            let v = match lex {
                "INF" => f64::INFINITY,
                "-INF" => f64::NEG_INFINITY,
                other => other.parse().expect("double lexical"),
            };
            T::Dbl(lex.to_string(), v)
        }
        _ => panic!("unexpected datatype in rendered term: {}", s),
    }
}

/// The partial SPARQL ordering the engine's total ORDER BY order must refine:
/// coarse ranks — unbound < blank < IRI < literal, and WITHIN literals the
/// engine's documented KIND-FIRST rank (numeric < boolean < string < langString,
/// the substrate `LiteralKind` order of sq-wjl8i; promoted from "unconstrained" by
/// sq-ilweo) — plus within-kind orders where the spec/probes define them. Returns
/// per-class monotonicity violations.
fn check_sorted(keys: &[Option<T>], desc: bool, ctx: &str) -> Result<(), String> {
    let oriented = |o: std::cmp::Ordering| if desc { o.reverse() } else { o };
    // (a) coarse rank sequence must be monotone.
    let rank = |k: &Option<T>| match k {
        None => 0u8,
        Some(T::BNode(_)) => 1,
        Some(T::Iri(_)) => 2,
        Some(T::Int(_) | T::Dec(..) | T::Dbl(..)) => 3,
        Some(T::Bool(_)) => 4,
        Some(T::Str(_)) => 5,
        Some(T::Lang(..)) => 6,
    };
    let mut prev_rank: Option<u8> = None;
    for k in keys {
        let r = rank(k);
        if let Some(p) = prev_rank {
            if oriented(p.cmp(&r)) == std::cmp::Ordering::Greater {
                return Err(format!("rank order violated ({} then {}) in {}", p, r, ctx));
            }
        }
        prev_rank = Some(r);
    }
    // (b) within-class subsequences must be monotone where the order is defined.
    let check_sub = |label: &str, extract: &dyn Fn(&Option<T>) -> Option<Vec<u8>>| -> Result<(), String> {
        let mut prev: Option<Vec<u8>> = None;
        for k in keys {
            if let Some(key) = extract(k) {
                if let Some(p) = &prev {
                    if oriented(p.cmp(&key)) == std::cmp::Ordering::Greater {
                        return Err(format!("{} subsequence order violated in {}", label, ctx));
                    }
                }
                prev = Some(key);
            }
        }
        Ok(())
    };
    check_sub("iri", &|k| match k {
        Some(T::Iri(i)) => Some(i.as_bytes().to_vec()),
        _ => None,
    })?;
    check_sub("xsd:string", &|k| match k {
        Some(T::Str(s)) => Some(s.as_bytes().to_vec()),
        _ => None,
    })?;
    check_sub("boolean", &|k| match k {
        Some(T::Bool(b)) => Some(vec![u8::from(*b)]),
        _ => None,
    })?;
    // langStrings sort LEX-FIRST (probe-pinned engine extension: the strict
    // comparator decides same-tag pairs by value and the cross-tag fallback
    // compares the lexical only) — so the LEXICAL subsequence must be monotone;
    // the tag on a lexical tie is engine-defined and stays unconstrained.
    check_sub("langString-lexical", &|k| match k {
        Some(T::Lang(s, _)) => Some(s.as_bytes().to_vec()),
        _ => None,
    })?;
    // numerics: compare by exact value — encode as a monotone-comparable check via
    // pairwise walk (values are not byte-encodable, so walk directly).
    let mut prev_num: Option<Num> = None;
    for k in keys {
        if let Some(n) = k.as_ref().and_then(num_of) {
            if let Some(p) = &prev_num {
                if oriented(num_cmp(p, &n)) == std::cmp::Ordering::Greater {
                    return Err(format!("numeric subsequence order violated in {}", ctx));
                }
            }
            prev_num = Some(n);
        }
    }
    Ok(())
}

fn is_sub_multiset(sub: &[String], full: &[String]) -> bool {
    // both sorted
    let mut it = full.iter();
    'outer: for s in sub {
        for f in it.by_ref() {
            match f.cmp(s) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => continue 'outer,
                std::cmp::Ordering::Greater => return false,
            }
        }
        return false;
    }
    true
}

/// The full family-1 check for one (data, query) case against one engine path.
fn check_case(data: &[[T; 3]], q: &Q, label: &str, rows: &[Vec<Option<String>>]) -> Result<(), TestCaseError> {
    let oracle = oracle_eval(data, q);
    let en_ms = multiset(rows);
    let or_ms = multiset(&oracle);
    let ctx = || {
        format!(
            "path={} query={:?}\ndata:\n{}engine rows: {:?}\noracle rows: {:?}",
            label,
            q.render(),
            to_ntriples(data),
            rows,
            oracle
        )
    };
    match (q.offset.unwrap_or(0), q.limit) {
        (0, None) => prop_assert_eq!(&en_ms, &or_ms, "multiset mismatch: {}", ctx()),
        (j, l) => {
            // §2.2 cardinality-only slice model (sq-ilweo widened to OFFSET):
            // OFFSET drops exactly j rows (saturating), LIMIT then caps at k;
            // WHICH rows survive is implementation-defined without a total order,
            // but every surviving row must come from the oracle multiset.
            let expected = {
                let after_offset = or_ms.len().saturating_sub(j);
                l.map_or(after_offset, |k| after_offset.min(k))
            };
            prop_assert_eq!(rows.len(), expected, "LIMIT/OFFSET cardinality mismatch: {}", ctx());
            prop_assert!(is_sub_multiset(&en_ms, &or_ms), "sliced rows not a sub-multiset: {}", ctx());
        }
    }
    if let Some((v, desc)) = q.order.first() {
        let col = q.project.iter().position(|p| p == v).expect("order var projected");
        let keys: Vec<Option<T>> = rows.iter().map(|r| r[col].as_deref().map(parse_rendered)).collect();
        if let Err(e) = check_sorted(&keys, *desc, &ctx()) {
            return Err(TestCaseError::fail(e));
        }
        if let Some((v2, desc2)) = q.order.get(1) {
            let col2 = q.project.iter().position(|p| p == v2).expect("order var projected");
            if let Err(e) = check_secondary_within_primary_runs(rows, col, col2, *desc2, &ctx()) {
                return Err(TestCaseError::fail(e));
            }
        }
    }
    Ok(())
}

/// Secondary ORDER BY key law (#2442): within each maximal run of rows whose
/// RENDERED primary cell is identical (unbound cells compare equal to each
/// other), the secondary-key sequence must satisfy `check_sorted`. Soundness:
/// identical rendered cells denote the same term, so a contiguous
/// identical-rendered run is a subsequence of one engine primary-tie group and
/// the engine must have ordered it by the secondary key. Value-equal ties with
/// DIFFERENT lexicals ("1"^^xsd:integer vs "1.0"^^xsd:decimal) fall outside the
/// runs and are deliberately not constrained — no cross-lexical tie modelling.
fn check_secondary_within_primary_runs(
    rows: &[Vec<Option<String>>],
    prim_col: usize,
    sec_col: usize,
    sec_desc: bool,
    ctx: &str,
) -> Result<(), String> {
    let mut start = 0;
    while start < rows.len() {
        let mut end = start + 1;
        while end < rows.len() && rows[end][prim_col] == rows[start][prim_col] {
            end += 1;
        }
        if end - start > 1 {
            let keys: Vec<Option<T>> = rows[start..end]
                .iter()
                .map(|r| r[sec_col].as_deref().map(parse_rendered))
                .collect();
            check_sorted(&keys, sec_desc, &format!("secondary key, primary-tie run [{}..{}) of {}", start, end, ctx))?;
        }
        start = end;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// Generators
// ═══════════════════════════════════════════════════════════════════════════════

const SUBJ_IRIS: [&str; 6] = [
    "http://ex/s0",
    "http://ex/s1",
    "http://ex/s2",
    "http://ex/s3",
    "http://ex/s4",
    "http://ex/s5",
];
const PRED_IRIS: [&str; 4] = ["http://ex/p0", "http://ex/p1", "http://ex/p2", "http://ex/p3"];
const BNODE_LABELS: [&str; 3] = ["b0", "b1", "b2"];
const STR_POOL: [&str; 6] = ["", "a", "b", "ab", "hi", "z9"];
const LANG_TAGS: [&str; 2] = ["en", "fr"];
/// Fixed double pool: (lexical, value). Finite values, ±INF, AND NaN (sq-ilweo
/// re-widened — computed-double lexicals and the NaN semantics are now modelled,
/// see the header).
const DBL_POOL: [(&str, f64); 9] = [
    ("0.0E0", 0.0),
    ("-0.0E0", -0.0),
    ("1.5E0", 1.5),
    ("-2.5E0", -2.5),
    ("1.0E2", 100.0),
    ("3.25E0", 3.25),
    ("INF", f64::INFINITY),
    ("-INF", f64::NEG_INFINITY),
    ("NaN", f64::NAN),
];

fn arb_literal() -> BoxedStrategy<T> {
    prop_oneof![
        (-30i64..=30).prop_map(T::Int),
        prop_oneof![Just(0i64), Just(1), Just(-1), Just(1000)].prop_map(T::Int),
        ((-3000i128..=3000), (1u32..=2)).prop_map(|(m, s)| T::Dec(m, s)),
        proptest::sample::select(&STR_POOL[..]).prop_map(|s| T::Str(s.to_string())),
        (proptest::sample::select(&STR_POOL[..]), proptest::sample::select(&LANG_TAGS[..]))
            .prop_map(|(s, t)| T::Lang(s.to_string(), t.to_string())),
        any::<bool>().prop_map(T::Bool),
        proptest::sample::select(&DBL_POOL[..]).prop_map(|(l, v)| T::Dbl(l.to_string(), v)),
    ]
    .boxed()
}

fn arb_subject() -> BoxedStrategy<T> {
    prop_oneof![
        4 => proptest::sample::select(&SUBJ_IRIS[..]).prop_map(|i| T::Iri(i.to_string())),
        1 => proptest::sample::select(&BNODE_LABELS[..]).prop_map(|b| T::BNode(b.to_string())),
    ]
    .boxed()
}

fn arb_triple() -> impl Strategy<Value = [T; 3]> {
    (
        arb_subject(),
        proptest::sample::select(&PRED_IRIS[..]).prop_map(|p| T::Iri(p.to_string())),
        prop_oneof![2 => arb_subject(), 3 => arb_literal()],
    )
        .prop_map(|(s, p, o)| [s, p, o])
}

fn arb_data(max: usize) -> impl Strategy<Value = Vec<[T; 3]>> {
    proptest::collection::vec(arb_triple(), 0..=max).prop_map(dedup_data)
}

fn arb_pos_s() -> impl Strategy<Value = Pos> {
    prop_oneof![
        3 => (0u8..=3).prop_map(Pos::Var),
        2 => proptest::sample::select(&SUBJ_IRIS[..]).prop_map(|i| Pos::Const(T::Iri(i.to_string()))),
    ]
}

fn arb_pos_p() -> impl Strategy<Value = Pos> {
    prop_oneof![
        1 => (3u8..=4).prop_map(Pos::Var),
        4 => proptest::sample::select(&PRED_IRIS[..]).prop_map(|p| Pos::Const(T::Iri(p.to_string()))),
    ]
}

fn arb_pos_o() -> impl Strategy<Value = Pos> {
    prop_oneof![
        4 => (0u8..=3).prop_map(Pos::Var),
        1 => proptest::sample::select(&SUBJ_IRIS[..]).prop_map(|i| Pos::Const(T::Iri(i.to_string()))),
        2 => arb_literal().prop_map(Pos::Const),
    ]
}

fn arb_tp() -> impl Strategy<Value = Tp> {
    (arb_pos_s(), arb_pos_p(), arb_pos_o()).prop_map(|(s, p, o)| Tp { s, p, o })
}

fn arb_bgp(min: usize, max: usize) -> impl Strategy<Value = Vec<Tp>> {
    proptest::collection::vec(arb_tp(), min..=max)
}

/// Force at least one variable so the SELECT projection is never empty:
/// descend to the leftmost BGP and variable-ize its first subject.
fn ensure_var(mut body: Body) -> Body {
    if body.vars().is_empty() {
        fn fix(b: &mut Body) {
            match b {
                Body::Bgp(tps) => tps[0].s = Pos::Var(0),
                Body::Opt(l, _) | Body::Union(l, _) => fix(l),
            }
        }
        fix(&mut body);
    }
    body
}

/// A sub-body under a top-level Opt/Union: a flat BGP (the pre-#2442 shape) or
/// one more level of Opt/Union over single-pattern BGPs — giving nested shapes
/// like `A OPTIONAL { B OPTIONAL { C } }` and `{ {A} UNION {B} } OPTIONAL { C }`.
fn arb_sub_body() -> impl Strategy<Value = Body> {
    let leaf = |min: usize, max: usize| arb_bgp(min, max).prop_map(Body::Bgp).boxed();
    prop_oneof![
        2 => leaf(1, 2),
        1 => (leaf(1, 1), leaf(1, 1)).prop_map(|(l, r)| Body::Opt(Box::new(l), Box::new(r))),
        1 => (leaf(1, 1), leaf(1, 1)).prop_map(|(a, b)| Body::Union(Box::new(a), Box::new(b))),
    ]
}

fn arb_body() -> impl Strategy<Value = Body> {
    prop_oneof![
        3 => arb_bgp(1, 3).prop_map(Body::Bgp),
        1 => (arb_sub_body(), arb_sub_body()).prop_map(|(l, r)| Body::Opt(Box::new(l), Box::new(r))),
        1 => (arb_sub_body(), arb_sub_body()).prop_map(|(a, b)| Body::Union(Box::new(a), Box::new(b))),
    ]
    .prop_map(ensure_var)
}

/// Expression constants: IRIs or literals (blank nodes are not legal in expressions).
fn arb_expr_const() -> impl Strategy<Value = T> {
    prop_oneof![
        1 => proptest::sample::select(&SUBJ_IRIS[..]).prop_map(|i| T::Iri(i.to_string())),
        3 => arb_literal(),
    ]
}

/// Leaf/inner expression seeds; resolved against the query's variable lists in
/// `build_expr` so generation composes without nested `prop_flat_map` towers.
#[derive(Clone, Debug)]
enum ExprSeed {
    EqVV(u8, u8),
    EqVC(u8, T),
    LtVK(u8, i64),
    GtVK(u8, i64),
    LtVV(u8, u8),
    GtVV(u8, u8),
    Bound(u8),
    Is(u8, u8), // (which of the four is* forms, var seed) — any in-scope var (sq-qeltv widened)
    Not(Box<ExprSeed>),
    And(Box<ExprSeed>, Box<ExprSeed>),
    Or(Box<ExprSeed>, Box<ExprSeed>),
}

fn arb_expr_seed() -> impl Strategy<Value = ExprSeed> {
    let leaf = prop_oneof![
        (any::<u8>(), any::<u8>()).prop_map(|(a, b)| ExprSeed::EqVV(a, b)),
        (any::<u8>(), arb_expr_const()).prop_map(|(v, c)| ExprSeed::EqVC(v, c)),
        (any::<u8>(), -5i64..=15).prop_map(|(v, k)| ExprSeed::LtVK(v, k)),
        (any::<u8>(), -5i64..=15).prop_map(|(v, k)| ExprSeed::GtVK(v, k)),
        (any::<u8>(), any::<u8>()).prop_map(|(a, b)| ExprSeed::LtVV(a, b)),
        (any::<u8>(), any::<u8>()).prop_map(|(a, b)| ExprSeed::GtVV(a, b)),
        any::<u8>().prop_map(ExprSeed::Bound),
        (0u8..4, any::<u8>()).prop_map(|(w, v)| ExprSeed::Is(w, v)),
    ];
    leaf.prop_recursive(2, 8, 2, |inner| {
        prop_oneof![
            inner.clone().prop_map(|e| ExprSeed::Not(Box::new(e))),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| ExprSeed::And(Box::new(a), Box::new(b))),
            (inner.clone(), inner).prop_map(|(a, b)| ExprSeed::Or(Box::new(a), Box::new(b))),
        ]
    })
}

/// Resolve seed var indices modulo the available variable list: `filter_vars` is
/// every var the FILTER may mention. The `is*` forms range over the SAME full list —
/// possibly-unbound OPTIONAL/UNION/BIND vars included — since the engine now returns
/// the SPARQL 17.2 type error for `is*(unbound)` exactly like the oracle (sq-qeltv).
fn build_expr(seed: &ExprSeed, filter_vars: &[V]) -> Expr {
    let pick = |s: u8, pool: &[V]| pool[s as usize % pool.len()];
    match seed {
        ExprSeed::EqVV(a, b) => Expr::EqVV(pick(*a, filter_vars), pick(*b, filter_vars)),
        ExprSeed::EqVC(v, c) => Expr::EqVC(pick(*v, filter_vars), c.clone()),
        ExprSeed::LtVK(v, k) => Expr::LtVK(pick(*v, filter_vars), *k),
        ExprSeed::GtVK(v, k) => Expr::GtVK(pick(*v, filter_vars), *k),
        ExprSeed::LtVV(a, b) => Expr::LtVV(pick(*a, filter_vars), pick(*b, filter_vars)),
        ExprSeed::GtVV(a, b) => Expr::GtVV(pick(*a, filter_vars), pick(*b, filter_vars)),
        ExprSeed::Bound(v) => Expr::Bound(pick(*v, filter_vars)),
        ExprSeed::Is(which, v) => {
            let var = pick(*v, filter_vars);
            match which % 4 {
                0 => Expr::IsIri(var),
                1 => Expr::IsBlank(var),
                2 => Expr::IsLit(var),
                _ => Expr::IsNum(var),
            }
        }
        ExprSeed::Not(e) => Expr::Not(Box::new(build_expr(e, filter_vars))),
        ExprSeed::And(a, b) => Expr::And(
            Box::new(build_expr(a, filter_vars)),
            Box::new(build_expr(b, filter_vars)),
        ),
        ExprSeed::Or(a, b) => Expr::Or(
            Box::new(build_expr(a, filter_vars)),
            Box::new(build_expr(b, filter_vars)),
        ),
    }
}

/// Independent raw seeds assembled into a valid query by `build_query` — this keeps
/// the strategy flat (no nested `prop_flat_map` towers) while every choice still
/// shrinks toward simpler queries.
#[derive(Clone, Debug)]
struct QSeed {
    body: Body,
    bind: Option<(u8, u8, i64)>, // (form, var seed, k)
    filter: Option<ExprSeed>,
    distinct: bool,
    proj_mask: u16,
    /// 1–2 ORDER BY key seeds when present: (projected-var seed, desc).
    order: Option<Vec<(u8, bool)>>,
    limit: Option<usize>,
    offset: Option<usize>,
}

fn arb_qseed() -> impl Strategy<Value = QSeed> {
    (
        arb_body(),
        proptest::option::weighted(0.35, (0u8..3, any::<u8>(), 0i64..=3)),
        proptest::option::weighted(0.45, arb_expr_seed()),
        proptest::bool::weighted(0.25),
        any::<u16>(),
        proptest::option::weighted(
            0.35,
            proptest::collection::vec((any::<u8>(), any::<bool>()), 1..=2),
        ),
        proptest::option::weighted(0.2, 0usize..=8),
        proptest::option::weighted(0.2, 0usize..=5),
    )
        .prop_map(|(body, bind, filter, distinct, proj_mask, order, limit, offset)| QSeed {
            body,
            bind,
            filter,
            distinct,
            proj_mask,
            order,
            limit,
            offset,
        })
}

fn build_query(seed: QSeed) -> Q {
    let body_vars = seed.body.vars();
    let bind = seed.bind.map(|(form, vs, k)| {
        let v = body_vars[vs as usize % body_vars.len()];
        match form % 3 {
            0 => Bind::PlusK(v, k),
            1 => Bind::LtK(v, k),
            _ => Bind::StrLenOf(v),
        }
    });
    let mut scope = body_vars.clone();
    if bind.is_some() {
        scope.push(BIND_VAR);
    }
    // FILTER may mention any in-scope var (including the BIND target: BIND
    // precedes FILTER in the rendered group, so it is visible there).
    let filter = seed.filter.as_ref().map(|f| build_expr(f, &scope));
    // Projection: the mask-selected subset of in-scope vars; never empty.
    let mut project: Vec<V> = scope
        .iter()
        .enumerate()
        .filter(|(i, _)| seed.proj_mask & (1 << (i % 16)) != 0)
        .map(|(_, v)| *v)
        .collect();
    if project.is_empty() {
        project = scope.clone();
    }
    // Resolve each order-key seed to a projected var, deduping on the var: a
    // repeated key (`ORDER BY ?x DESC(?x)`) is legal but vacuous — the second
    // key would never break a tie of the first — so keep distinct vars only.
    let mut order: Vec<(V, bool)> = Vec::new();
    for (vs, desc) in seed.order.iter().flatten() {
        let v = project[*vs as usize % project.len()];
        if !order.iter().any(|(ov, _)| *ov == v) {
            order.push((v, *desc));
        }
    }
    Q {
        body: seed.body,
        bind,
        filter,
        distinct: seed.distinct,
        project,
        order,
        limit: seed.limit,
        offset: seed.offset,
    }
}

/// Pool of subject-position terms for pattern instantiation.
fn subject_pool() -> Vec<T> {
    let mut out: Vec<T> = SUBJ_IRIS.iter().map(|i| T::Iri(i.to_string())).collect();
    out.extend(BNODE_LABELS.iter().map(|b| T::BNode(b.to_string())));
    out
}

/// Pool of object-position terms for pattern instantiation.
fn object_pool() -> Vec<T> {
    let mut out = subject_pool();
    out.extend([-2i64, 0, 1, 3, 17].map(T::Int));
    out.extend([T::Dec(150, 2), T::Dec(-25, 2), T::Dec(40, 1)]);
    out.extend(STR_POOL.iter().map(|s| T::Str(s.to_string())));
    out.push(T::Lang("a".to_string(), "en".to_string()));
    out.extend([T::Bool(true), T::Bool(false)]);
    out.extend(DBL_POOL.iter().map(|(l, v)| T::Dbl(l.to_string(), *v)));
    out
}

/// Instantiate every pattern of `body` under ONE shared variable assignment decoded
/// from `seed` — the injected triples then satisfy each BGP simultaneously, so
/// generated cases produce nonempty (and join-exercising) results at a useful rate
/// instead of comparing empty-vs-empty (the `generator_diversity_floor` guard).
/// Variables are classified by the strictest position they occupy: predicate
/// position needs a predicate IRI, subject position an IRI/bnode, object-only
/// positions may take any term.
fn instantiate(body: &Body, seed: &[u8]) -> Vec<[T; 3]> {
    let vars = body.vars();
    fn collect_tps<'a>(b: &'a Body, out: &mut Vec<&'a Tp>) {
        match b {
            Body::Bgp(tps) => out.extend(tps.iter()),
            Body::Opt(l, r) | Body::Union(l, r) => {
                collect_tps(l, out);
                collect_tps(r, out);
            }
        }
    }
    let mut all_tps: Vec<&Tp> = Vec::new();
    collect_tps(body, &mut all_tps);
    let position_class = |v: &V| {
        let mut class = 2u8; // 0 = predicate, 1 = subject, 2 = object-only
        for tp in &all_tps {
            if tp.p == Pos::Var(*v) {
                class = 0;
            } else if tp.s == Pos::Var(*v) && class > 1 {
                class = 1;
            }
        }
        class
    };
    let subj = subject_pool();
    let obj = object_pool();
    let assignment: Vec<(V, T)> = vars
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let s = *seed.get(i).unwrap_or(&0) as usize;
            let t = match position_class(v) {
                0 => T::Iri(PRED_IRIS[s % PRED_IRIS.len()].to_string()),
                1 => subj[s % subj.len()].clone(),
                _ => obj[s % obj.len()].clone(),
            };
            (*v, t)
        })
        .collect();
    let resolve = |p: &Pos| match p {
        Pos::Const(t) => t.clone(),
        Pos::Var(v) => assignment.iter().find(|(av, _)| av == v).expect("assigned var").1.clone(),
    };
    all_tps.iter().map(|tp| [resolve(&tp.s), resolve(&tp.p), resolve(&tp.o)]).collect()
}

/// A full family-1 case: the graph is a blend of independent random triples and
/// 0..=2 shared-assignment instantiations of the query's own patterns (see
/// `instantiate`).
fn arb_case() -> impl Strategy<Value = (Vec<[T; 3]>, Q)> {
    arb_qseed().prop_map(build_query).prop_flat_map(|q| {
        let nvars = q.body.vars().len();
        (
            arb_data(14),
            proptest::collection::vec(proptest::collection::vec(any::<u8>(), nvars), 0..=2),
            Just(q),
        )
            .prop_map(move |(mut data, seeds, q)| {
                for seed in &seeds {
                    data.extend(instantiate(&q.body, seed));
                }
                (dedup_data(data), q)
            })
    })
}

// ═══════════════════════════════════════════════════════════════════════════════
// Family 1: engine ≡ naive reference (every compiled plan path)
// ═══════════════════════════════════════════════════════════════════════════════

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        ..ProptestConfig::default()
    })]

    /// Core oracle property: for a random graph and a random query, the engine's
    /// solution multiset equals the naive reference's, on every compiled plan path;
    /// ORDER BY output additionally respects the SPARQL ordering; LIMIT is checked
    /// as sub-multiset + cardinality (implementation-defined slice, §2.2).
    #[test]
    fn family1_engine_matches_naive_reference((data, q) in arb_case()) {
        let g = Graph::load_str(&to_ntriples(&data), "ntriples").expect("generated N-Triples parse");
        let qtext = q.render();
        for (label, rows) in engine_paths(&g, &qtext) {
            check_case(&data, &q, label, &rows)?;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Family 2: join-order permutation invariance
// ═══════════════════════════════════════════════════════════════════════════════

fn arb_perm_case() -> impl Strategy<Value = (Vec<[T; 3]>, Vec<Tp>, Vec<Tp>, Option<ExprSeed>)> {
    (arb_bgp(2, 4).prop_map(|b| match ensure_var(Body::Bgp(b)) { Body::Bgp(t) => t, _ => unreachable!() }))
        .prop_flat_map(|tps| {
            let nvars = Body::Bgp(tps.clone()).vars().len();
            (
                arb_data(14),
                proptest::collection::vec(proptest::collection::vec(any::<u8>(), nvars), 0..=2),
                Just(tps.clone()),
                Just(tps).prop_shuffle(),
                proptest::option::weighted(0.4, arb_expr_seed()),
            )
                .prop_map(|(mut data, seeds, tps, shuffled, fseed)| {
                    for seed in &seeds {
                        data.extend(instantiate(&Body::Bgp(tps.clone()), seed));
                    }
                    (dedup_data(data), tps, shuffled, fseed)
                })
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// Permuting the triple patterns of a BGP (with an optional FILTER on top) must
    /// not change the solution multiset — the planner may reorder joins for cost
    /// only. Checked engine-vs-engine on every compiled plan path.
    #[test]
    fn family2_join_order_invariance((data, tps, shuffled, fseed) in arb_perm_case()) {
        let build = |patterns: Vec<Tp>| {
            let body = Body::Bgp(patterns);
            let vars = body.vars();
            let filter = fseed.as_ref().map(|f| build_expr(f, &vars));
            Q { body, bind: None, filter, distinct: false, project: vars, order: vec![], limit: None, offset: None }
        };
        let q1 = build(tps);
        let q2 = build(shuffled);
        let g = Graph::load_str(&to_ntriples(&data), "ntriples").expect("generated N-Triples parse");
        for (label, rows1) in engine_paths(&g, &q1.render()) {
            let rows2 = engine_rows(&g, &q2.render());
            prop_assert_eq!(
                multiset(&rows1), multiset(&rows2),
                "join-order variance on path {}:\n{}\nvs\n{}\ndata:\n{}",
                label, q1.render(), q2.render(), to_ntriples(&data)
            );
        }
        // and both agree with the oracle (ties family 2 to the independent spec).
        for (label, rows1) in engine_paths(&g, &q1.render()) {
            check_case(&data, &q1, label, &rows1)?;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Family 3: ORDER BY laws on tie-free integer keys (exact sequences)
// ═══════════════════════════════════════════════════════════════════════════════

fn arb_orderby_case() -> impl Strategy<Value = (Vec<(usize, i64)>, usize, usize)> {
    (
        proptest::collection::btree_set(-50i64..=50, 1..=12),
        0usize..=14,
        0usize..=14,
    )
        .prop_map(|(vals, k, j)| {
            let pairs: Vec<(usize, i64)> = vals.into_iter().enumerate().collect();
            (pairs, k, j)
        })
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        ..ProptestConfig::default()
    })]

    /// With pairwise-distinct integer sort keys the order is total, so exact
    /// sequence laws hold: ASC equals the oracle's sort, DESC is its exact
    /// reverse, `LIMIT k` is the exact k-prefix, `OFFSET j` is the exact j-suffix,
    /// and `LIMIT k OFFSET j` is the exact window (sq-ilweo).
    #[test]
    fn family3_orderby_exact_sequence_laws((pairs, k, j) in arb_orderby_case()) {
        // one triple per distinct integer value; subjects may repeat.
        let data: Vec<[T; 3]> = pairs
            .iter()
            .map(|(i, v)| {
                [T::Iri(SUBJ_IRIS[i % SUBJ_IRIS.len()].to_string()), T::Iri(PRED_IRIS[0].to_string()), T::Int(*v)]
            })
            .collect();
        let g = Graph::load_str(&to_ntriples(&data), "ntriples").expect("generated N-Triples parse");
        let render_row = |(i, v): &(usize, i64)| {
            vec![
                Some(format!("<{}>", SUBJ_IRIS[i % SUBJ_IRIS.len()])),
                Some(T::Int(*v).render()),
            ]
        };
        let mut expected_asc: Vec<Vec<Option<String>>> = pairs.iter().map(render_row).collect();
        expected_asc.sort_by_key(|row| {
            // sort by the integer value parsed back from the rendered cell — pairs
            // are already value-sorted (BTreeSet), this keeps the oracle honest.
            match parse_rendered(row[1].as_deref().unwrap()) {
                T::Int(n) => n,
                other => panic!("non-integer key {:?}", other),
            }
        });
        let base = format!("SELECT ?s ?o WHERE {{ ?s <{}> ?o }}", PRED_IRIS[0]);
        for (label, asc) in engine_paths(&g, &format!("{} ORDER BY ?o", base)) {
            prop_assert_eq!(&asc, &expected_asc, "ASC exact sequence, path {}", label);
        }
        let mut expected_desc = expected_asc.clone();
        expected_desc.reverse();
        for (label, desc) in engine_paths(&g, &format!("{} ORDER BY DESC(?o)", base)) {
            prop_assert_eq!(&desc, &expected_desc, "DESC is exact reverse of ASC, path {}", label);
        }
        let prefix: Vec<_> = expected_asc.iter().take(k).cloned().collect();
        for (label, lim) in engine_paths(&g, &format!("{} ORDER BY ?o LIMIT {}", base, k)) {
            prop_assert_eq!(&lim, &prefix, "ORDER BY + LIMIT is the exact prefix, path {}", label);
        }
        let suffix: Vec<_> = expected_asc.iter().skip(j).cloned().collect();
        for (label, off) in engine_paths(&g, &format!("{} ORDER BY ?o OFFSET {}", base, j)) {
            prop_assert_eq!(&off, &suffix, "ORDER BY + OFFSET is the exact suffix, path {}", label);
        }
        let window: Vec<_> = expected_asc.iter().skip(j).take(k).cloned().collect();
        for (label, win) in engine_paths(&g, &format!("{} ORDER BY ?o LIMIT {} OFFSET {}", base, k, j)) {
            prop_assert_eq!(&win, &window, "ORDER BY + LIMIT + OFFSET is the exact window, path {}", label);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Oracle known-answer pins (a degenerate oracle cannot silently agree)
// ═══════════════════════════════════════════════════════════════════════════════

/// OPTIONAL + shared-var join, hand-computed. Data:
///   s0 p0 s1 ; s1 p0 s2 ; s0 p1 "a"
/// Query: SELECT ?v0 ?v1 ?v2 WHERE { ?v0 <p0> ?v1 OPTIONAL { ?v1 <p0> ?v2 } }
/// Expected rows: (s0, s1, s2) and (s1, s2, unbound).
#[test]
fn oracle_known_answer_optional() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let data = vec![
        [s(0), p(0), s(1)],
        [s(1), p(0), s(2)],
        [s(0), p(1), T::Str("a".to_string())],
    ];
    let q = Q {
        body: Body::Opt(
            Box::new(Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }])),
            Box::new(Body::Bgp(vec![Tp { s: Pos::Var(1), p: Pos::Const(p(0)), o: Pos::Var(2) }])),
        ),
        bind: None,
        filter: None,
        distinct: false,
        project: vec![0, 1, 2],
        order: vec![],
        limit: None,
        offset: None,
    };
    let expected = vec![
        vec![Some(s(0).render()), Some(s(1).render()), Some(s(2).render())],
        vec![Some(s(1).render()), Some(s(2).render()), None],
    ];
    assert_eq!(multiset(&oracle_eval(&data, &q)), multiset(&expected), "oracle vs hand-computed");
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    assert_eq!(multiset(&engine_rows(&g, &q.render())), multiset(&expected), "engine vs hand-computed");
}

/// UNION + DISTINCT + subset projection + FILTER, hand-computed. Data:
///   s0 p0 1 ; s1 p0 2 ; s0 p1 1
/// Query: SELECT DISTINCT ?v0 WHERE { { ?v0 <p0> ?v1 } UNION { ?v0 <p1> ?v1 }
///                                    FILTER((?v1 < 2)) }
/// Branch rows: (s0,1) (s1,2) | (s0,1); filter keeps ?v1=1 rows; projection to ?v0
/// gives s0, s0; DISTINCT gives one s0.
#[test]
fn oracle_known_answer_union_distinct_filter() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let data = vec![
        [s(0), p(0), T::Int(1)],
        [s(1), p(0), T::Int(2)],
        [s(0), p(1), T::Int(1)],
    ];
    let q = Q {
        body: Body::Union(
            Box::new(Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }])),
            Box::new(Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(1)), o: Pos::Var(1) }])),
        ),
        bind: None,
        filter: Some(Expr::LtVK(1, 2)),
        distinct: true,
        project: vec![0],
        order: vec![],
        limit: None,
        offset: None,
    };
    let expected = vec![vec![Some(s(0).render())]];
    assert_eq!(multiset(&oracle_eval(&data, &q)), multiset(&expected), "oracle vs hand-computed");
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    assert_eq!(multiset(&engine_rows(&g, &q.render())), multiset(&expected), "engine vs hand-computed");
    // without DISTINCT the duplicate survives — pins multiset (not set) semantics.
    let q_bag = Q { distinct: false, ..q };
    let expected_bag = vec![vec![Some(s(0).render())], vec![Some(s(0).render())]];
    assert_eq!(multiset(&oracle_eval(&data, &q_bag)), multiset(&expected_bag), "bag semantics");
    assert_eq!(multiset(&engine_rows(&g, &q_bag.render())), multiset(&expected_bag), "engine bag semantics");
}

/// BIND forms, hand-computed: PlusK over int/decimal/error, LtK boolean/error,
/// STRLEN(STR()) over IRI and literals.
#[test]
fn oracle_known_answer_bind_forms() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let data = vec![
        [s(0), p(0), T::Int(41)],
        [s(1), p(0), T::Dec(150, 2)],
        [s(2), p(0), T::Str("hi".to_string())],
    ];
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    let mk = |bind: Bind| Q {
        body: Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }]),
        bind: Some(bind),
        filter: None,
        distinct: false,
        project: vec![0, 1, BIND_VAR],
        order: vec![],
        limit: None,
        offset: None,
    };
    for q in [mk(Bind::PlusK(1, 1)), mk(Bind::LtK(1, 42)), mk(Bind::StrLenOf(1))] {
        let oracle = oracle_eval(&data, &q);
        let engine = engine_rows(&g, &q.render());
        assert_eq!(multiset(&engine), multiset(&oracle), "BIND {:?}", q.bind);
    }
    // Spot-pin exact computed values so the oracle's arithmetic is itself checked:
    let q = mk(Bind::PlusK(1, 1));
    let oracle = oracle_eval(&data, &q);
    let cells: std::collections::HashSet<String> =
        oracle.iter().map(|r| r[2].clone().unwrap_or_else(|| "UNBOUND".to_string())).collect();
    assert!(cells.contains(&format!("\"42\"^^<{}integer>", XSD)), "int + 1");
    assert!(cells.contains(&format!("\"2.50\"^^<{}decimal>", XSD)), "decimal scale preserved");
    assert!(cells.contains("UNBOUND"), "type-error BIND leaves the var unbound");
}

/// Computed-double lexical rules, hand-computed (sq-ilweo — the "pin the engine's
/// rules FIRST" half of the widening): a double + int BIND promotes to xsd:double
/// and renders per the arithmetic convention — specials keep their XSD spellings,
/// an integral value prints as a PLAIN integer, anything else as shortest
/// mantissa-E-exponent with ≥ 1 fraction digit. The oracle derives every cell
/// through `fmt_double_oracle`, so engine ≡ oracle here proves the independent
/// formatter reproduces the engine's rules on every special/integral/fractional
/// shape the pool can produce.
#[test]
fn oracle_known_answer_computed_double() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let d = |lex: &str, v: f64| T::Dbl(lex.to_string(), v);
    let data = vec![
        [s(0), p(0), d("1.5E0", 1.5)],
        [s(1), p(0), d("1.0E2", 100.0)],
        [s(2), p(0), d("-2.5E0", -2.5)],
        [s(3), p(0), d("-0.0E0", -0.0)],
        [s(4), p(0), d("INF", f64::INFINITY)],
        [s(5), p(0), d("NaN", f64::NAN)],
    ];
    let q = Q {
        body: Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }]),
        bind: Some(Bind::PlusK(1, 1)),
        filter: None,
        distinct: false,
        project: vec![0, 1, BIND_VAR],
        order: vec![],
        limit: None,
        offset: None,
    };
    let oracle = oracle_eval(&data, &q);
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    assert_eq!(multiset(&engine_rows(&g, &q.render())), multiset(&oracle), "engine vs oracle");
    let cells: std::collections::HashSet<String> =
        oracle.iter().map(|r| r[2].clone().expect("computed double is bound")).collect();
    for expect in ["2.5E0", "101", "-1.5E0", "1", "INF", "NaN"] {
        assert!(
            cells.contains(&format!("\"{}\"^^<{}double>", expect, XSD)),
            "computed double cell {} missing from {:?}",
            expect,
            cells
        );
    }
    // The single-digit-mantissa shape (-2.5 + 2 = -0.5 → "-5.0E-1", exercising the
    // mandatory fraction digit) is reachable but RARE in generation — pin it
    // end-to-end too.
    let q2 = Q { bind: Some(Bind::PlusK(1, 2)), ..q };
    let oracle2 = oracle_eval(&data, &q2);
    assert_eq!(multiset(&engine_rows(&g, &q2.render())), multiset(&oracle2), "engine vs oracle, k=2");
    let cells2: std::collections::HashSet<String> =
        oracle2.iter().map(|r| r[2].clone().expect("computed double is bound")).collect();
    assert!(
        cells2.contains(&format!("\"-5.0E-1\"^^<{}double>", XSD)),
        "-2.5 + 2 must render with the mandatory fraction digit: {:?}",
        cells2
    );
}

/// NaN value semantics, hand-computed (sq-ilweo): `=` against the NaN constant is
/// sameTerm-TRUE for the stored NaN term and FALSE (not an error!) for any other
/// numeric — pinned by the NEGATED filter, which keeps exactly the false rows;
/// `<` against a numeric constant is a TYPE ERROR (not false!) — pinned by the
/// negated filter dropping the NaN row (`!err = err`); ORDER BY totalises NaN
/// FIRST, before -INF.
#[test]
fn oracle_known_answer_nan() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let nan = || T::Dbl("NaN".to_string(), f64::NAN);
    let data = vec![
        [s(0), p(0), nan()],
        [s(1), p(0), T::Dbl("5.0E0".to_string(), 5.0)],
        [s(2), p(0), T::Dbl("-INF".to_string(), f64::NEG_INFINITY)],
    ];
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    let mk = |filter: Expr| Q {
        body: Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }]),
        bind: None,
        filter: Some(filter),
        distinct: false,
        project: vec![0],
        order: vec![],
        limit: None,
        offset: None,
    };
    let row = |i: usize| vec![Some(s(i).render())];
    let cases: Vec<(Expr, Vec<Vec<Option<String>>>)> = vec![
        // ?v = NaN keeps exactly the identical NaN term (sameTerm fast path).
        (Expr::EqVC(1, nan()), vec![row(0)]),
        // !(?v = NaN) keeps the OTHER numerics: their verdict is false, not error.
        (Expr::Not(Box::new(Expr::EqVC(1, nan()))), vec![row(1), row(2)]),
        // ?v < 7 drops NaN as a TYPE ERROR and keeps the comparable rows.
        (Expr::LtVK(1, 7), vec![row(1), row(2)]),
        // !(?v < 7) drops EVERYTHING: !err = err for NaN, !true = false for the rest.
        (Expr::Not(Box::new(Expr::LtVK(1, 7))), vec![]),
    ];
    for (filter, expected) in cases {
        let q = mk(filter);
        assert_eq!(multiset(&oracle_eval(&data, &q)), multiset(&expected), "oracle: {}", q.render());
        assert_eq!(multiset(&engine_rows(&g, &q.render())), multiset(&expected), "engine: {}", q.render());
    }
    // ORDER BY: NaN totalised FIRST — before -INF (tie-free, exact sequence).
    let base = format!("SELECT ?o WHERE {{ ?s <{}> ?o }} ORDER BY ?o", PRED_IRIS[0]);
    let expected_asc: Vec<Vec<Option<String>>> = [nan(), T::Dbl("-INF".to_string(), f64::NEG_INFINITY), T::Dbl("5.0E0".to_string(), 5.0)]
        .iter()
        .map(|t| vec![Some(t.render())])
        .collect();
    for (label, asc) in engine_paths(&g, &base) {
        assert_eq!(asc, expected_asc, "NaN-first ORDER BY, path {}", label);
    }
}

/// Var-var relational `<` / `>` semantics, hand-computed (issue #2443): the strict
/// comparator decides only same-family pairs — numeric×numeric by value across
/// tiers, string×string by codepoint, boolean false < true, and the LENIENT
/// same-tag langString extension (`FILTER("a"@en < "b"@en)` is TRUE, and the
/// reversed pair is FALSE — not an error, pinned via the NEGATED filter). A
/// cross-tag langString pair, a langString×string pair, a NaN operand, and an
/// IRI pair are all TYPE ERRORS: dropped by the filter AND by its negation.
#[test]
fn oracle_known_answer_var_var_relational() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let b = |i: usize| T::BNode(BNODE_LABELS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let lang = |l: &str, t: &str| T::Lang(l.to_string(), t.to_string());
    // one (?v1, ?v2) operand pair per subject, via two shared-subject patterns.
    let pairs: Vec<(T, T, T)> = vec![
        (s(0), lang("a", "en"), lang("b", "en")), // same-tag lang: Less (the extension)
        (s(1), lang("b", "en"), lang("a", "en")), // same-tag lang: Greater (false on <, NOT an error)
        (s(2), lang("a", "en"), lang("b", "fr")), // cross-tag lang: type error
        (s(3), lang("a", "en"), T::Str("b".to_string())), // lang × string: type error
        (s(4), T::Str("a".to_string()), T::Str("b".to_string())), // string codepoint: Less
        (s(5), T::Int(1), T::Dec(150, 2)),        // numeric by value across tiers: 1 < 1.50
        (b(0), T::Bool(false), T::Bool(true)),    // boolean: false < true
        (b(1), T::Dbl("NaN".to_string(), f64::NAN), T::Int(0)), // NaN operand: type error
        (b(2), s(0), s(1)),                       // IRI pair: not value-comparable, type error
    ];
    let mut data = Vec::new();
    for (subj, left, right) in &pairs {
        data.push([subj.clone(), p(0), left.clone()]);
        data.push([subj.clone(), p(1), right.clone()]);
    }
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    let mk = |filter: Expr| Q {
        body: Body::Bgp(vec![
            Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) },
            Tp { s: Pos::Var(0), p: Pos::Const(p(1)), o: Pos::Var(2) },
        ]),
        bind: None,
        filter: Some(filter),
        distinct: false,
        project: vec![0],
        order: None,
        limit: None,
        offset: None,
    };
    let row = |t: &T| vec![Some(t.render())];
    let less_subjects = || vec![row(&s(0)), row(&s(4)), row(&s(5)), row(&b(0))];
    let cases: Vec<(Expr, Vec<Vec<Option<String>>>)> = vec![
        // ?v1 < ?v2 keeps exactly the decided-Less pairs — including "a"@en < "b"@en.
        (Expr::LtVV(1, 2), less_subjects()),
        // !(?v1 < ?v2) keeps ONLY the decided-Greater pair: every type-error pair
        // (cross-tag lang, lang×string, NaN, IRIs) stays dropped (!err = err).
        (Expr::Not(Box::new(Expr::LtVV(1, 2))), vec![row(&s(1))]),
        // ?v1 > ?v2 keeps the decided-Greater pair,
        (Expr::GtVV(1, 2), vec![row(&s(1))]),
        // and its negation keeps the decided-Less pairs.
        (Expr::Not(Box::new(Expr::GtVV(1, 2))), less_subjects()),
    ];
    for (filter, expected) in cases {
        let q = mk(filter);
        assert_eq!(multiset(&oracle_eval(&data, &q)), multiset(&expected), "oracle: {}", q.render());
        for (label, rows) in engine_paths(&g, &q.render()) {
            assert_eq!(multiset(&rows), multiset(&expected), "engine ({}): {}", label, q.render());
        }
    }
}

/// The literal KIND-FIRST rank and the langString lex-first order, hand-computed
/// (sq-ilweo promoted these from "unconstrained"; the order is the substrate's
/// documented sq-wjl8i `LiteralKind` rank): numeric < boolean < string <
/// langString, langStrings by lexical even across different tags.
#[test]
fn oracle_known_answer_literal_kind_rank_and_lang_order() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let objs = [
        T::Lang("b".to_string(), "en".to_string()),
        T::Str("z".to_string()),
        T::Int(5),
        T::Lang("a".to_string(), "fr".to_string()),
        T::Bool(true),
    ];
    let data: Vec<[T; 3]> = objs.iter().cloned().enumerate().map(|(i, o)| [s(i), p(0), o]).collect();
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    // numeric 5 < boolean true < string "z" < lang "a"@fr < lang "b"@en (lex-first
    // across tags) — tie-free, so the exact sequence is deterministic.
    let expected: Vec<Vec<Option<String>>> = [&objs[2], &objs[4], &objs[1], &objs[3], &objs[0]]
        .iter()
        .map(|t| vec![Some(t.render())])
        .collect();
    let base = format!("SELECT ?o WHERE {{ ?s <{}> ?o }} ORDER BY ?o", PRED_IRIS[0]);
    for (label, asc) in engine_paths(&g, &base) {
        assert_eq!(asc, expected, "kind-first ASC, path {}", label);
    }
    let mut expected_desc = expected.clone();
    expected_desc.reverse();
    let desc_q = format!("SELECT ?o WHERE {{ ?s <{}> ?o }} ORDER BY DESC(?o)", PRED_IRIS[0]);
    for (label, desc) in engine_paths(&g, &desc_q) {
        assert_eq!(desc, expected_desc, "kind-first DESC, path {}", label);
    }
    // And the family-1 sortedness checker accepts exactly this order and rejects
    // a kind-rank transposition (its own non-vacuity pin).
    let keys: Vec<Option<T>> = [&objs[2], &objs[4], &objs[1], &objs[3], &objs[0]].iter().map(|t| Some((*t).clone())).collect();
    assert!(check_sorted(&keys, false, "unit").is_ok());
    let bad: Vec<Option<T>> = vec![Some(objs[1].clone()), Some(objs[2].clone())]; // string before numeric
    assert!(check_sorted(&bad, false, "unit").is_err());
}

/// Nested OPTIONAL (#2442), hand-computed. Data:
///   s0 p0 s1 ; s1 p1 s2 ; s2 p2 "a" ; s3 p0 s4
/// Query: SELECT ?v0 ?v1 ?v2 ?v3
///        WHERE { ?v0 <p0> ?v1 OPTIONAL { ?v1 <p1> ?v2 OPTIONAL { ?v2 <p2> ?v3 } } }
/// LeftJoin(A, LeftJoin(B, C)): the (s0,s1) row extends through both inner
/// levels to (s0,s1,s2,"a"); the (s3,s4) row finds no compatible inner row and
/// survives with ?v2/?v3 unbound.
#[test]
fn oracle_known_answer_nested_optional() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let data = vec![
        [s(0), p(0), s(1)],
        [s(1), p(1), s(2)],
        [s(2), p(2), T::Str("a".to_string())],
        [s(3), p(0), s(4)],
    ];
    let q = Q {
        body: Body::Opt(
            Box::new(Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }])),
            Box::new(Body::Opt(
                Box::new(Body::Bgp(vec![Tp { s: Pos::Var(1), p: Pos::Const(p(1)), o: Pos::Var(2) }])),
                Box::new(Body::Bgp(vec![Tp { s: Pos::Var(2), p: Pos::Const(p(2)), o: Pos::Var(3) }])),
            )),
        ),
        bind: None,
        filter: None,
        distinct: false,
        project: vec![0, 1, 2, 3],
        order: vec![],
        limit: None,
        offset: None,
    };
    let expected = vec![
        vec![Some(s(0).render()), Some(s(1).render()), Some(s(2).render()), Some(T::Str("a".to_string()).render())],
        vec![Some(s(3).render()), Some(s(4).render()), None, None],
    ];
    assert_eq!(multiset(&oracle_eval(&data, &q)), multiset(&expected), "oracle vs hand-computed");
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    assert_eq!(multiset(&engine_rows(&g, &q.render())), multiset(&expected), "engine vs hand-computed");
}

/// UNION nested as OPTIONAL's left operand (#2442), hand-computed. Data:
///   s0 p0 s1 ; s0 p1 s2 ; s1 p2 "a"
/// Query: SELECT ?v0 ?v1 ?v2
///        WHERE { { { ?v0 <p0> ?v1 } UNION { ?v0 <p1> ?v1 } } OPTIONAL { ?v1 <p2> ?v2 } }
/// LeftJoin(Union(A, B), C): the p0 branch row (s0,s1) matches ?v1=s1 in C and
/// extends to (s0,s1,"a"); the p1 branch row (s0,s2) does not and keeps ?v2 unbound.
#[test]
fn oracle_known_answer_union_under_optional() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let data = vec![
        [s(0), p(0), s(1)],
        [s(0), p(1), s(2)],
        [s(1), p(2), T::Str("a".to_string())],
    ];
    let q = Q {
        body: Body::Opt(
            Box::new(Body::Union(
                Box::new(Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }])),
                Box::new(Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(1)), o: Pos::Var(1) }])),
            )),
            Box::new(Body::Bgp(vec![Tp { s: Pos::Var(1), p: Pos::Const(p(2)), o: Pos::Var(2) }])),
        ),
        bind: None,
        filter: None,
        distinct: false,
        project: vec![0, 1, 2],
        order: vec![],
        limit: None,
        offset: None,
    };
    let expected = vec![
        vec![Some(s(0).render()), Some(s(1).render()), Some(T::Str("a".to_string()).render())],
        vec![Some(s(0).render()), Some(s(2).render()), None],
    ];
    assert_eq!(multiset(&oracle_eval(&data, &q)), multiset(&expected), "oracle vs hand-computed");
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    assert_eq!(multiset(&engine_rows(&g, &q.render())), multiset(&expected), "engine vs hand-computed");
}

/// Multi-key ORDER BY (#2442) with a primary-key tie, hand-computed exact
/// sequence: `ORDER BY ?v1 DESC(?v0)` over { s0 p0 1 ; s1 p0 1 ; s2 p0 0 } —
/// the combined key is tie-free, so the exact row order is pinned on every
/// compiled plan path (also exercises the multi-key `Q::render` path).
#[test]
fn orderby_multikey_exact_sequence() {
    let s = |i: usize| T::Iri(SUBJ_IRIS[i].to_string());
    let p = |i: usize| T::Iri(PRED_IRIS[i].to_string());
    let data = vec![
        [s(0), p(0), T::Int(1)],
        [s(1), p(0), T::Int(1)],
        [s(2), p(0), T::Int(0)],
    ];
    let q = Q {
        body: Body::Bgp(vec![Tp { s: Pos::Var(0), p: Pos::Const(p(0)), o: Pos::Var(1) }]),
        bind: None,
        filter: None,
        distinct: false,
        project: vec![0, 1],
        order: vec![(1, false), (0, true)],
        limit: None,
        offset: None,
    };
    // ?v1 ascending (0 < 1), then within the ?v1=1 tie ?v0 descending (s1 > s0).
    let expected = vec![
        vec![Some(s(2).render()), Some(T::Int(0).render())],
        vec![Some(s(1).render()), Some(T::Int(1).render())],
        vec![Some(s(0).render()), Some(T::Int(1).render())],
    ];
    let g = Graph::load_str(&to_ntriples(&data), "ntriples").unwrap();
    for (label, rows) in engine_paths(&g, &q.render()) {
        assert_eq!(&rows, &expected, "multi-key exact sequence, path {}", label);
    }
    // The run-scoped secondary check accepts this ordering and rejects a swap
    // of the tied rows (the checker itself is non-vacuous on real data).
    assert!(check_secondary_within_primary_runs(&expected, 1, 0, true, "unit").is_ok());
    let mut swapped = expected.clone();
    swapped.swap(1, 2);
    assert!(check_secondary_within_primary_runs(&swapped, 1, 0, true, "unit").is_err());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Generator diversity floor (anti-vacuity: the corpus is not trivially empty)
// ═══════════════════════════════════════════════════════════════════════════════

/// Samples the family-1 case strategy with a FIXED seed (deterministic) and
/// asserts the corpus contains the situations the properties are supposed to
/// exercise. If a generator change starves the corpus, this fails loudly rather
/// than letting the properties pass vacuously.
#[test]
fn generator_diversity_floor() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::from_seed(RngAlgorithm::ChaCha, &[7u8; 32]),
    );
    let strategy = arb_case();
    let n = 300;
    let (mut nonempty, mut dups, mut unbound_cells, mut multi_row) = (0, 0, 0, 0);
    let (mut bgp, mut opt, mut union, mut filt, mut bind, mut distinct, mut order, mut limit, mut offset) =
        (0, 0, 0, 0, 0, 0, 0, 0, 0);
    // #2442 nesting/multi-key floors: bodies with a nested Opt/Union operand, and
    // ORDER BY over ≥ 2 keys.
    let (mut nested, mut order2) = (0, 0);
    // sq-ilweo widening floors: PlusK binds actually meeting a double in the data
    // (the computed-double path), and graphs actually containing the NaN term.
    let (mut plusk_dbl, mut nan_data) = (0, 0);
    for _ in 0..n {
        let (data, q) = strategy.new_tree(&mut runner).expect("gen").current();
        match &q.body {
            Body::Bgp(_) => bgp += 1,
            Body::Opt(l, r) | Body::Union(l, r) => {
                if matches!(q.body, Body::Opt(..)) {
                    opt += 1;
                } else {
                    union += 1;
                }
                nested += usize::from(
                    !matches!(l.as_ref(), Body::Bgp(_)) || !matches!(r.as_ref(), Body::Bgp(_)),
                );
            }
        }
        filt += usize::from(q.filter.is_some());
        bind += usize::from(q.bind.is_some());
        distinct += usize::from(q.distinct);
        order += usize::from(!q.order.is_empty());
        order2 += usize::from(q.order.len() >= 2);
        limit += usize::from(q.limit.is_some());
        offset += usize::from(q.offset.is_some());
        let has_dbl = data.iter().any(|t| t.iter().any(|x| matches!(x, T::Dbl(..))));
        plusk_dbl += usize::from(matches!(q.bind, Some(Bind::PlusK(..))) && has_dbl);
        nan_data += usize::from(data.iter().any(|t| t.iter().any(|x| matches!(x, T::Dbl(lex, _) if lex == "NaN"))));
        let rows = oracle_eval(&data, &q);
        nonempty += usize::from(!rows.is_empty());
        multi_row += usize::from(rows.len() > 1);
        let ms = multiset(&rows);
        dups += usize::from(ms.windows(2).any(|w| w[0] == w[1]));
        unbound_cells += usize::from(rows.iter().any(|r| r.iter().any(Option::is_none)));
    }
    eprintln!(
        "diversity: nonempty={}/{} multi_row={} dups={} unbound={} shapes: bgp={} opt={} union={} nested={} filter={} bind={} distinct={} order={} order2={} limit={} offset={} plusk_dbl={} nan_data={}",
        nonempty, n, multi_row, dups, unbound_cells, bgp, opt, union, nested, filt, bind, distinct, order, order2, limit, offset, plusk_dbl, nan_data
    );
    // Conservative floors — deterministic because the seed is fixed.
    assert!(nonempty * 100 >= n * 25, "nonempty results: {}/{}", nonempty, n);
    assert!(multi_row * 100 >= n * 15, "multi-row results: {}/{}", multi_row, n);
    assert!(dups >= 5, "duplicate-containing multisets: {}", dups);
    assert!(unbound_cells >= 5, "results with unbound cells: {}", unbound_cells);
    assert!(plusk_dbl >= 5, "PlusK-over-double cases: {}", plusk_dbl);
    assert!(nan_data >= 5, "NaN-containing graphs: {}", nan_data);
    for (label, count) in [
        ("bgp", bgp), ("optional", opt), ("union", union), ("filter", filt),
        ("bind", bind), ("distinct", distinct), ("order", order), ("limit", limit),
        ("offset", offset),
    ] {
        assert!(count >= 10, "query shape {} generated only {} times in {}", label, count, n);
    }
    // The #2442 widenings actually occur in the corpus (lower floors: each is a
    // sub-case of an already-floored shape).
    assert!(nested >= 8, "nested Opt/Union bodies generated only {} times in {}", nested, n);
    assert!(order2 >= 8, "two-key ORDER BY generated only {} times in {}", order2, n);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Unit pins for the comparison plumbing itself
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_rendered_roundtrips_every_generated_shape() {
    let terms = [
        T::Iri("http://ex/s0".to_string()),
        T::BNode("b1".to_string()),
        T::Str("hi".to_string()),
        T::Str(String::new()),
        T::Lang("a".to_string(), "en".to_string()),
        T::Int(-42),
        T::Dec(-25, 2),
        T::Dec(4000, 1),
        T::Dbl("1.5E0".to_string(), 1.5),
        T::Dbl("-INF".to_string(), f64::NEG_INFINITY),
        // sq-ilweo widened shapes: the NaN term (T's lexical PartialEq makes this
        // round-trip assertable) and a computed double's plain-integer lexical.
        T::Dbl("NaN".to_string(), f64::NAN),
        T::Dbl("101".to_string(), 101.0),
        T::Bool(true),
    ];
    for t in terms {
        assert_eq!(parse_rendered(&t.render()), t, "round-trip of {:?}", t);
    }
}

/// Pins the INDEPENDENT computed-double formatter against the engine's documented
/// lexical rules (the end-to-end agreement is pinned by
/// `oracle_known_answer_computed_double`): specials, plain integral (incl. -0.0 and
/// the 1e15 boundary), and shortest scientific with a mandatory fraction digit.
#[test]
fn fmt_double_oracle_matches_engine_rules() {
    assert_eq!(fmt_double_oracle(f64::NAN), "NaN");
    assert_eq!(fmt_double_oracle(f64::INFINITY), "INF");
    assert_eq!(fmt_double_oracle(f64::NEG_INFINITY), "-INF");
    assert_eq!(fmt_double_oracle(0.0), "0");
    assert_eq!(fmt_double_oracle(-0.0), "0");
    assert_eq!(fmt_double_oracle(101.0), "101");
    assert_eq!(fmt_double_oracle(-101.0), "-101");
    assert_eq!(fmt_double_oracle(2.5), "2.5E0");
    assert_eq!(fmt_double_oracle(-1.5), "-1.5E0");
    assert_eq!(fmt_double_oracle(0.2), "2.0E-1");
    assert_eq!(fmt_double_oracle(101.5), "1.015E2");
    assert_eq!(fmt_double_oracle(1e15), "1.0E15");
    assert_eq!(fmt_double_oracle(999999999999999.0), "999999999999999");
}

#[test]
fn check_sorted_detects_violations() {
    // an inverted numeric pair must be flagged...
    let bad = vec![Some(T::Int(2)), Some(T::Int(1))];
    assert!(check_sorted(&bad, false, "unit").is_err());
    // ...but is fine descending.
    assert!(check_sorted(&bad, true, "unit").is_ok());
    // unbound must come first ascending,
    let bad2 = vec![Some(T::Int(1)), None];
    assert!(check_sorted(&bad2, false, "unit").is_err());
    // cross-kind literal pairs follow the KIND-FIRST rank (sq-ilweo promoted from
    // "unconstrained"): a string before a numeric violates ascending order but is
    // exactly right descending,
    let ranked = vec![Some(T::Str("z".to_string())), Some(T::Int(1))];
    assert!(check_sorted(&ranked, false, "unit").is_err());
    assert!(check_sorted(&ranked, true, "unit").is_ok());
    // as does a boolean before a numeric,
    let ranked2 = vec![Some(T::Bool(false)), Some(T::Int(5))];
    assert!(check_sorted(&ranked2, false, "unit").is_err());
    assert!(check_sorted(&ranked2, true, "unit").is_ok());
    // langStrings order lex-first even across tags; a tag tie on equal lexicals is
    // engine-defined (unconstrained in both directions),
    let lang_bad = vec![
        Some(T::Lang("b".to_string(), "en".to_string())),
        Some(T::Lang("a".to_string(), "fr".to_string())),
    ];
    assert!(check_sorted(&lang_bad, false, "unit").is_err());
    assert!(check_sorted(&lang_bad, true, "unit").is_ok());
    let lang_tie = vec![
        Some(T::Lang("a".to_string(), "fr".to_string())),
        Some(T::Lang("a".to_string(), "en".to_string())),
    ];
    assert!(check_sorted(&lang_tie, false, "unit").is_ok());
    assert!(check_sorted(&lang_tie, true, "unit").is_ok());
    // NaN is totalised FIRST among numerics (sq-ilweo),
    let nan = || Some(T::Dbl("NaN".to_string(), f64::NAN));
    assert!(check_sorted(&[nan(), Some(T::Dbl("-INF".to_string(), f64::NEG_INFINITY))], false, "unit").is_ok());
    assert!(check_sorted(&[Some(T::Int(-100)), nan()], false, "unit").is_err());
    assert!(check_sorted(&[nan(), nan()], false, "unit").is_ok());
    // and value-equal cross-type numerics are a tie, not a violation.
    let tie = vec![Some(T::Dec(10, 1)), Some(T::Int(1))];
    assert!(check_sorted(&tie, false, "unit").is_ok());
}

#[test]
fn secondary_run_check_detects_violations() {
    let row = |prim: Option<i64>, sec: i64| {
        vec![prim.map(|v| T::Int(v).render()), Some(T::Int(sec).render())]
    };
    // an inverted secondary pair inside one primary run must be flagged...
    let bad = vec![row(Some(1), 2), row(Some(1), 1)];
    assert!(check_secondary_within_primary_runs(&bad, 0, 1, false, "unit").is_err());
    // ...but is fine descending.
    assert!(check_secondary_within_primary_runs(&bad, 0, 1, true, "unit").is_ok());
    // across DIFFERENT primary cells the secondary is unconstrained.
    let free = vec![row(Some(1), 2), row(Some(2), 1)];
    assert!(check_secondary_within_primary_runs(&free, 0, 1, false, "unit").is_ok());
    // unbound primary cells compare equal to each other and form a run.
    let ub = vec![row(None, 5), row(None, 3), row(Some(1), 0)];
    assert!(check_secondary_within_primary_runs(&ub, 0, 1, false, "unit").is_err());
    // value-equal but lexically-different primaries ("1" int vs "1.0" decimal)
    // break the run: deliberately unconstrained (no cross-lexical tie modelling).
    let lex = vec![
        vec![Some(T::Int(1).render()), Some(T::Int(2).render())],
        vec![Some(T::Dec(10, 1).render()), Some(T::Int(1).render())],
    ];
    assert!(check_secondary_within_primary_runs(&lex, 0, 1, false, "unit").is_ok());
}

#[test]
fn render_dec_matches_engine_scale_rules() {
    assert_eq!(render_dec(250, 2), "2.50");
    assert_eq!(render_dec(-25, 2), "-0.25");
    assert_eq!(render_dec(40, 1), "4.0");
    assert_eq!(render_dec(0, 2), "0.00");
    assert_eq!(render_dec(-3000, 2), "-30.00");
}
