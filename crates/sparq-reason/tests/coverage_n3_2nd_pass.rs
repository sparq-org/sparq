//! Direct unit tests for sq-qcnn.36 — second-pass coverage targeting uncovered lines
//! in `n3/mod.rs` and `incremental.rs`:
//!
//! * Trig inverse reverse modes (lines 2022-2034)
//! * Negation F64/forward/non-numeric paths (lines 2005, 2010, 2011, 2592-2595)
//! * String predicates: containsRoughly, equalIgnoringCase, notEqualIgnoringCase
//! * String format `%f` and excess-arg failure
//! * String concatenation decimal/double coercions
//! * f64 arithmetic path with double inputs
//! * eval_exact decimal arithmetic (quotient/product/negation/abs/floor/ceil/exp)
//! * datetime InSeconds with negative UTC offset
//! * Fallback serialisation: lang-tagged literals, special-char literals, blank nodes
//! * `MaterializedN3Graph::is_empty`
//! * `reason_n3_terms_with_resolver` with an explicit base IRI
//!
//! 🤖 SPARQ agent — sq-qcnn.36 2nd-pass.

use sparq_reason::n3::{reason_n3_terms_with_resolver, Term};
use sparq_reason::{reason_n3_terms, MaterializedN3Graph, N3Mode};

// ---- helpers ----------------------------------------------------------------

fn iri(s: &str) -> Term {
    Term::Iri(s.into())
}
fn ex(local: &str) -> Term {
    iri(&format!("http://ex/{local}"))
}

const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DEC: &str = "http://www.w3.org/2001/XMLSchema#decimal";

const PRE: &str = "@prefix : <http://ex/> .\n\
    @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
    @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
    @prefix list: <http://www.w3.org/2000/10/swap/list#> .\n\
    @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
    @prefix time: <http://www.w3.org/2000/10/swap/time#> .\n\
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";

fn run(body: &str) -> Vec<[Term; 3]> {
    reason_n3_terms(&format!("{PRE}{body}"), None)
        .expect("N3 reasoning failed")
        .facts
}

fn has(facts: &[[Term; 3]], s: &Term, p: &Term, o: &Term) -> bool {
    facts.iter().any(|f| &f[0] == s && &f[1] == p && &f[2] == o)
}
fn has_sp(facts: &[[Term; 3]], s: &Term, p: &Term) -> bool {
    facts.iter().any(|f| &f[0] == s && &f[1] == p)
}
fn typed_lit(v: &str, dt: &str) -> Term {
    Term::Lit(v.into(), dt.into(), None)
}

fn r_fired(facts: &[[Term; 3]], local: &str) -> bool {
    has(facts, &ex("r"), &ex(local), &ex("fired"))
}
fn r_pred(facts: &[[Term; 3]], local: &str) -> bool {
    has_sp(facts, &ex("r"), &ex(local))
}

// ---- trig inverse reverse modes ---------------------------------------------

/// `?t math:cos/tan/asin/…/degrees/radians <value>` triggers the reverse path
/// in `eval_functional` (lines 2022-2034): the inverse function is applied to
/// the bound object to bind the subject.
#[test]
fn trig_inverse_reverse_modes() {
    let fired = |builtin: &str, val: &str| -> bool {
        let src = format!("{PRE}{{ ?t math:{builtin} {val} }} => {{ :r :{builtin} :fired }} .");
        let c = reason_n3_terms(&src, None).expect("reasoning ok").facts;
        c.iter().any(|f| f[0] == ex("r") && f[1] == ex(builtin) && f[2] == ex("fired"))
    };

    assert!(fired("cos", "0.5"), "math:cos reverse (acos) — line 2022");
    assert!(fired("tan", "1.0"), "math:tan reverse (atan) — line 2023");
    assert!(fired("asin", "0.5"), "math:asin reverse (sin) — line 2024");
    assert!(fired("acos", "0.5"), "math:acos reverse (cos) — line 2025");
    assert!(fired("atan", "0.5"), "math:atan reverse (tan) — line 2026");
    assert!(fired("sinh", "1.0"), "math:sinh reverse (asinh) — line 2027");
    assert!(fired("cosh", "1.5"), "math:cosh reverse (acosh) — line 2028");
    assert!(fired("tanh", "0.5"), "math:tanh reverse (atanh) — line 2029");
    assert!(fired("asinh", "1.0"), "math:asinh reverse (sinh) — line 2030");
    assert!(fired("acosh", "1.0"), "math:acosh reverse (cosh) — line 2031");
    assert!(fired("atanh", "0.5"), "math:atanh reverse (tanh) — line 2032");
    assert!(fired("degrees", "1.0"), "math:degrees reverse — line 2033");
    assert!(fired("radians", "1.0"), "math:radians reverse — line 2034");
}

/// `?t math:sin 2.0` — asin(2.0) is NaN, so the premise fails closed (line 2047).
/// `?t math:cos :v` — non-numeric object, falls to `return None` (lines 2051-2053).
#[test]
fn trig_inverse_nan_and_non_numeric_fail() {
    // asin(2.0) = NaN: premise must fail (line 2047)
    let c = run("{ ?t math:sin 2.0 } => { :r :asin2 :fired } .");
    assert!(!r_fired(&c, "asin2"), "asin(2) is NaN — premise must fail; got {:?}", c);

    // non-numeric object: numval returns None, falls through to return None (lines 2051-2053)
    let c = run("{ ?t math:cos :notANumber } => { :r :cosnop :fired } .");
    assert!(
        !r_fired(&c, "cosnop"),
        "cos with IRI object must fail; got {:?}",
        c
    );
}

// ---- negation: F64 reverse, forward paths, non-numeric fail -----------------

/// math:negation paths not covered by the existing reverse/decimal tests.
///
/// * F64 reverse (line 2005): `?x math:negation 1.5e0`
/// * Non-numeric object (line 2010): `?x math:negation :iri`
/// * Bound subject forward — integer (lines 2011, 2592, 2593)
/// * Forward decimal (line 2594)
/// * Forward F64 (lines 2595, 2416): eval_exact declines F64, falls to f64 path
#[test]
fn negation_f64_reverse_and_forward_paths() {
    // F64 reverse: `NumVal::F64(x) => NumVal::F64(-x)` — line 2005
    let c = run("{ ?x math:negation 1.5e0 } => { :r :f64rev :fired } .");
    assert!(r_fired(&c, "f64rev"), "negation F64 reverse must fire; got {:?}", c);

    // non-numeric object: `numval` fails → `return None` — line 2010
    let c = run("{ ?x math:negation :notnum } => { :r :nonnumobj :fired } .");
    assert!(
        !r_fired(&c, "nonnumobj"),
        "negation with IRI object must fail; got {:?}",
        c
    );

    // forward integer: literal subject `3`, unbound object — lines 2592, 2593
    let c = run("{ 3 math:negation ?y } => { :r :fwdint ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("fwdint"), &typed_lit("-3", XSD_INT)),
        "forward integer negation: -3; got {:?}",
        c
    );

    // forward decimal: literal subject `2.5` — line 2594
    let c = run("{ 2.5 math:negation ?y } => { :r :fwddec ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("fwddec"), &typed_lit("-2.5", XSD_DEC)),
        "forward decimal negation: -2.5; got {:?}",
        c
    );

    // forward F64: eval_exact declines (`NumVal::F64(_) => return None`, line 2595),
    // falls to f64 path (`Func::Negation => -nums[0]`, line 2416)
    let c = run("{ 1.5e0 math:negation ?y } => { :r :fwdf64 :fired } .");
    assert!(r_fired(&c, "fwdf64"), "forward F64 negation must fire; got {:?}", c);

    // bound-subject forward via a data fact: exercises line 2011 (closing brace
    // of `if !s_applied.is_ground()`) when `?n` is already bound.
    let c = run(":a :v 5 .\n{ :a :v ?n . ?n math:negation ?result } => { :r :bound ?result } .");
    assert!(
        has(&c, &ex("r"), &ex("bound"), &typed_lit("-5", XSD_INT)),
        "negation with bound subject: -5; got {:?}",
        c
    );
}

// ---- absoluteValue decimal + F64 paths --------------------------------------

/// AbsoluteValue decimal input → eval_exact line 2599;
/// F64 input → eval_exact declines (line 2600), f64 path line 2417.
#[test]
fn abs_decimal_and_f64_paths() {
    let c = run("{ -2.5 math:absoluteValue ?y } => { :r :absdec ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("absdec"), &typed_lit("2.5", XSD_DEC)),
        "absoluteValue decimal: 2.5; got {:?}",
        c
    );

    let c = run("{ -1.5e0 math:absoluteValue ?y } => { :r :absf64 :fired } .");
    assert!(r_fired(&c, "absf64"), "absoluteValue F64 must fire; got {:?}", c);
}

// ---- string builtins: containsRoughly, equalIgnoringCase, notEqualIgnoringCase -

/// Lines 1368-1375 in `eval_builtin`.
#[test]
fn string_predicates_roughly_and_case() {
    let sp = "@prefix : <http://ex/> .\n\
              @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n";

    // string:containsRoughly (lines 1368-1369) — case-insensitive substring
    let c = reason_n3_terms(
        &format!("{sp}{{ \"Hello World\" string:containsRoughly \"hello\" }} => {{ :r :cr :fired }} ."),
        None,
    )
    .expect("ok")
    .facts;
    assert!(
        c.iter().any(|f| f[0] == ex("r") && f[1] == ex("cr") && f[2] == ex("fired")),
        "containsRoughly must fire; got {:?}",
        c
    );

    // negative case
    let c2 = reason_n3_terms(
        &format!("{sp}{{ \"Hello\" string:containsRoughly \"xyz\" }} => {{ :r :crneg :fired }} ."),
        None,
    )
    .expect("ok")
    .facts;
    assert!(
        !c2.iter().any(|f| f[1] == ex("crneg")),
        "containsRoughly must NOT fire; got {:?}",
        c2
    );

    // string:equalIgnoringCase (lines 1370-1371)
    let c3 = reason_n3_terms(
        &format!("{sp}{{ \"ABC\" string:equalIgnoringCase \"abc\" }} => {{ :r :eic :fired }} ."),
        None,
    )
    .expect("ok")
    .facts;
    assert!(
        c3.iter().any(|f| f[1] == ex("eic")),
        "equalIgnoringCase must fire; got {:?}",
        c3
    );

    // string:notEqualIgnoringCase (lines 1373-1375)
    let c4 = reason_n3_terms(
        &format!(
            "{sp}{{ \"ABC\" string:notEqualIgnoringCase \"xyz\" }} => {{ :r :neic :fired }} ."
        ),
        None,
    )
    .expect("ok")
    .facts;
    assert!(
        c4.iter().any(|f| f[1] == ex("neic")),
        "notEqualIgnoringCase must fire; got {:?}",
        c4
    );
}

// ---- string:format %f and excess-arg failure --------------------------------

/// Lines 2147-2149 (%f path), 2155 (extra-arg failure).
#[test]
fn string_format_percent_f_and_excess_args() {
    // %f: C-style fixed-point 6 decimal places — line 2147-2149
    let c = run(
        "{ ( \"%f\" 3.14 ) string:format ?s } => { :r :fmt ?s } .",
    );
    assert!(r_pred(&c, "fmt"), "format %%f must bind a string; got {:?}", c);
    // verify the result contains the decimal separator
    let v = c
        .iter()
        .find(|f| f[0] == ex("r") && f[1] == ex("fmt"))
        .map(|f| &f[2]);
    if let Some(Term::Lit(s, _, _)) = v {
        assert!(s.contains('.'), "%%f result should contain '.'; got {s:?}");
    }

    // excess args (argi != args.len()): line 2155
    let c2 = run("{ ( \"%s\" \"a\" \"extra\" ) string:format ?s } => { :r :fmtx :fired } .");
    assert!(
        !r_fired(&c2, "fmtx"),
        "excess args must fail the premise; got {:?}",
        c2
    );
}

// ---- string:concatenation decimal and double coercions ----------------------

/// Lines 2178, 2182-2183, 2188-2193, 2195 in `Func::Concat`.
#[test]
fn string_concat_numeric_coercions() {
    // integer coercion (line 2178): `5` as "5"
    let c = run("{ ( \"x\" 5 ) string:concatenation ?s } => { :r :cint ?s } .");
    assert!(
        has(&c, &ex("r"), &ex("cint"), &typed_lit("x5", "http://www.w3.org/2001/XMLSchema#string")),
        "concat integer must give 'x5'; got {:?}",
        c
    );

    // decimal integer-valued: `3.0` → scale=0 path (line 2182)
    let c = run("{ ( \"y\" 3.0 ) string:concatenation ?s } => { :r :cdec0 ?s } .");
    assert!(r_pred(&c, "cdec0"), "concat decimal 3.0 must fire; got {:?}", c);

    // decimal non-integer: `2.5` → lex path (line 2183-2185)
    let c = run("{ ( \"z\" 2.5 ) string:concatenation ?s } => { :r :cdec ?s } .");
    assert!(r_pred(&c, "cdec"), "concat decimal 2.5 must fire; got {:?}", c);

    // F64 integer-valued: `2.0e0` → integer format (line 2189-2190)
    let c = run("{ ( \"a\" 2.0e0 ) string:concatenation ?s } => { :r :cf64i ?s } .");
    assert!(r_pred(&c, "cf64i"), "concat F64 2.0e0 must fire; got {:?}", c);

    // F64 non-integer: `1.5e0` → non-integer format (lines 2191-2192)
    let c = run("{ ( \"b\" 1.5e0 ) string:concatenation ?s } => { :r :cf64f ?s } .");
    assert!(r_pred(&c, "cf64f"), "concat F64 1.5e0 must fire; got {:?}", c);
}

// ---- f64 arithmetic path with double inputs ---------------------------------

/// Lines 2374-2376 (Product/Max/Min), 2378-2379 (Difference), 2389-2390
/// (Exponentiation), 2408 (Remainder returns None), 2410-2414
/// (IntegerQuotient f64), 2416-2420 (unary ops), 2426 (Atan), 2430-2432
/// (Asinh/Acosh/Atanh), 2447 (number_term decimal path for non-exact quotient),
/// 2771 (number_term format branch).
#[test]
fn f64_path_with_double_inputs() {
    // Product of two doubles — line 2374
    let c = run("{ ( 2.0e0 3.0e0 ) math:product ?y } => { :r :prod ?y } .");
    assert!(r_pred(&c, "prod"), "double product must fire; got {:?}", c);

    // Max — line 2375
    let c = run("{ ( 1.0e0 3.0e0 ) math:max ?y } => { :r :mx ?y } .");
    assert!(r_pred(&c, "mx"), "double max must fire; got {:?}", c);

    // Min — line 2376
    let c = run("{ ( 1.0e0 3.0e0 ) math:min ?y } => { :r :mn ?y } .");
    assert!(r_pred(&c, "mn"), "double min must fire; got {:?}", c);

    // Difference — lines 2378-2379
    let c = run("{ ( 5.0e0 2.0e0 ) math:difference ?y } => { :r :diff ?y } .");
    assert!(r_pred(&c, "diff"), "double difference must fire; got {:?}", c);

    // Exponentiation — lines 2389-2390
    let c = run("{ ( 2.0e0 3.0e0 ) math:exponentiation ?y } => { :r :exp ?y } .");
    assert!(r_pred(&c, "exp"), "double exponentiation must fire; got {:?}", c);

    // Remainder on doubles returns None (integer-only) — line 2408
    let c = run("{ ( 2.0e0 3.0e0 ) math:remainder ?y } => { :r :remf :fired } .");
    assert!(
        !r_fired(&c, "remf"),
        "remainder on doubles must fail (integer-only); got {:?}",
        c
    );

    // Remainder with 3 args (wrong count) → eval_exact line 2575, f64 line 2408
    let c = run("{ ( 1 2 3 ) math:remainder ?y } => { :r :remn :fired } .");
    assert!(!r_fired(&c, "remn"), "3-arg remainder must fail; got {:?}", c);

    // IntegerQuotient zero-divisor: eval_exact line 2588, f64 lines 2410-2412
    let c = run("{ ( 5 0 ) math:integerQuotient ?y } => { :r :iqz :fired } .");
    assert!(!r_fired(&c, "iqz"), "integerQuotient /0 must fail; got {:?}", c);

    // IntegerQuotient non-zero double inputs: f64 lines 2413-2414
    let c = run("{ ( 10.0e0 3.0e0 ) math:integerQuotient ?y } => { :r :iqdbl ?y } .");
    assert!(r_pred(&c, "iqdbl"), "double integerQuotient must fire; got {:?}", c);

    // Negation, AbsoluteValue, Rounded, Floor, Ceiling on doubles — lines 2416-2420;
    // eval_exact declines F64 first
    for (builtin, label) in [
        ("negation", "neg"),
        ("absoluteValue", "abs"),
        ("rounded", "rnd"),
        ("floor", "flr"),
        ("ceiling", "ceil"),
    ] {
        let c = run(&format!(
            "{{ 1.5e0 math:{builtin} ?y }} => {{ :r :{label} :fired }} ."
        ));
        assert!(r_fired(&c, label), "double {builtin} must fire; got {:?}", c);
    }

    // Atan forward (trig-family double) — line 2426
    let c = run("{ 1.0e0 math:atan ?y } => { :r :atanf :fired } .");
    assert!(r_fired(&c, "atanf"), "double atan must fire; got {:?}", c);

    // Asinh, Acosh, Atanh — lines 2430-2432
    let c = run("{ 1.0e0 math:asinh ?y } => { :r :asinhf :fired } .");
    assert!(r_fired(&c, "asinhf"), "double asinh must fire; got {:?}", c);
    let c = run("{ 1.5e0 math:acosh ?y } => { :r :acoshf :fired } .");
    assert!(r_fired(&c, "acoshf"), "double acosh must fire; got {:?}", c);
    let c = run("{ 0.5e0 math:atanh ?y } => { :r :atanhf :fired } .");
    assert!(r_fired(&c, "atanhf"), "double atanh must fire; got {:?}", c);

    // Non-exact integer quotient: eval_exact non-exact loop + number_term non-integer
    // f64 result — lines 2555-2562, 2447, 2771
    let c = run("{ ( 10 3 ) math:quotient ?y } => { :r :inxq :fired } .");
    assert!(r_fired(&c, "inxq"), "non-exact quotient (10/3) must fire; got {:?}", c);

    // Atan2 with y=0 returns None — line 2404
    let c = run("{ ( 1.0 0.0 ) math:atan2 ?y } => { :r :at2z :fired } .");
    assert!(!r_fired(&c, "at2z"), "atan2 with y=0 must fail; got {:?}", c);
}

// ---- eval_exact decimal arithmetic ------------------------------------------

/// Exercises the exact decimal tower: Sum, Product (decimal inputs, lines 2490,
/// 2527), Quotient (exact + inexact), Floor/Ceiling on decimal, Negation/Abs
/// decimal, Exponentiation (lines 2617-2640).
#[test]
fn eval_exact_decimal_arithmetic() {
    // Decimal sum triggers renorm with any_dec=true — line 2490
    let c = run("{ ( 1.5 2.0 ) math:sum ?y } => { :r :dsum ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("dsum"), &typed_lit("3.5", XSD_DEC)),
        "decimal sum 1.5+2.0=3.5; got {:?}",
        c
    );

    // Decimal product (Dec arm for v, line 2527)
    let c = run("{ ( 3.0 2.5 ) math:product ?y } => { :r :dprod ?y } .");
    assert!(r_pred(&c, "dprod"), "decimal product 3.0*2.5 must fire; got {:?}", c);

    // Decimal product with F64 first arg: eval_exact F64 arm (line 2528) → falls
    // to f64 path (line 2374)
    let c = run("{ ( 2.0e0 3.0e0 ) math:product ?y } => { :r :f64prod :fired } .");
    assert!(r_fired(&c, "f64prod"), "F64 product must fire; got {:?}", c);

    // eval_exact quotient: exact integer division (scale=0) — line 2567
    let c = run("{ ( 6 2 ) math:quotient ?y } => { :r :iqex ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("iqex"), &typed_lit("3", XSD_INT)),
        "6/2=3 exact integer quotient; got {:?}",
        c
    );

    // eval_exact quotient: exact decimal division — line 2565
    let c = run("{ ( 1 4 ) math:quotient ?y } => { :r :dqex ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("dqex"), &typed_lit("0.25", XSD_DEC)),
        "1/4=0.25 exact decimal quotient; got {:?}",
        c
    );

    // eval_exact quotient: remainder of 2.5/3 is non-integer → _ arm (line 2582)
    let c = run("{ ( 2.5 3 ) math:remainder ?y } => { :r :remdc :fired } .");
    assert!(
        !r_fired(&c, "remdc"),
        "decimal remainder must fail (integer-only); got {:?}",
        c
    );

    // eval_exact Negation forward: integer path (lines 2592-2593)
    let c = run("{ 7 math:negation ?y } => { :r :negfwd ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("negfwd"), &typed_lit("-7", XSD_INT)),
        "forward negation 7 -> -7; got {:?}",
        c
    );

    // eval_exact Negation decimal (line 2594)
    let c = run("{ 3.5 math:negation ?y } => { :r :negdec ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("negdec"), &typed_lit("-3.5", XSD_DEC)),
        "forward decimal negation 3.5 -> -3.5; got {:?}",
        c
    );

    // eval_exact AbsoluteValue decimal (line 2599)
    let c = run("{ -4.5 math:absoluteValue ?y } => { :r :absdec2 ?y } .");
    assert!(r_pred(&c, "absdec2"), "decimal absoluteValue must fire; got {:?}", c);

    // eval_exact Floor/Ceiling on integer — lines 2607-2608, 2611-2612
    let c = run("{ 3.7 math:floor ?y } => { :r :flr ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("flr"), &typed_lit("3", XSD_INT)),
        "floor(3.7) = 3; got {:?}",
        c
    );
    let c = run("{ 3.2 math:ceiling ?y } => { :r :ceil ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("ceil"), &typed_lit("4", XSD_INT)),
        "ceiling(3.2) = 4; got {:?}",
        c
    );

    // eval_exact Floor with integer input — unary_int Int arm (line 2499)
    let c = run("{ 3 math:floor ?y } => { :r :flrint ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("flrint"), &typed_lit("3", XSD_INT)),
        "floor(3) = 3; got {:?}",
        c
    );

    // unary_int F64 arm (line 2501): eval_exact declines, f64 path floor (line 2419)
    let c = run("{ 3.7e0 math:floor ?y } => { :r :flrf64 :fired } .");
    assert!(r_fired(&c, "flrf64"), "F64 floor must fire; got {:?}", c);

    // eval_exact Exponentiation: integer^integer (lines 2617-2640)
    let c = run("{ ( 2 10 ) math:exponentiation ?y } => { :r :expii ?y } .");
    assert!(
        has(&c, &ex("r"), &ex("expii"), &typed_lit("1024", XSD_INT)),
        "2^10 = 1024; got {:?}",
        c
    );

    // eval_exact Exponentiation: decimal base (lines 2624-2626)
    let c = run("{ ( 2.0 3 ) math:exponentiation ?y } => { :r :expdec ?y } .");
    assert!(r_pred(&c, "expdec"), "2.0^3 must fire; got {:?}", c);

    // eval_exact Exponentiation: exponent out of range (>64) — eval_exact returns None
    // (line 2622), falls to f64 path which still succeeds
    let c = run("{ ( 2 65 ) math:exponentiation ?y } => { :r :expoor :fired } .");
    assert!(r_fired(&c, "expoor"), "2^65 falls to f64 path and fires; got {:?}", c);

    // wrong arg count (3 args) → eval_exact line 2619, f64 two() fails → rule absent
    let c = run("{ ( 2 3 4 ) math:exponentiation ?y } => { :r :expwrong :fired } .");
    assert!(!r_fired(&c, "expwrong"), "3-arg exponentiation must fail; got {:?}", c);
}

// ---- datetime InSeconds with timezone offset --------------------------------

/// Lines 2685 (negative-offset time stripping), 2699-2705 (UTC offset parsing).
#[test]
fn datetime_inseconds_with_negative_utc_offset() {
    // "2024-01-01T15:00:00-05:00"^^xsd:dateTime in seconds =
    // 2024-01-01T20:00:00Z = 1704153600
    let c = run(concat!(
        "{ \"2024-01-01T15:00:00-05:00\"^^xsd:dateTime time:inSeconds ?s } ",
        "=> { :r :tz ?s } ."
    ));
    assert!(r_pred(&c, "tz"), "inSeconds with negative offset must fire; got {:?}", c);

    // positive offset also exercises the timezone path
    let c = run(concat!(
        "{ \"2024-01-01T07:30:00+05:30\"^^xsd:dateTime time:inSeconds ?s } ",
        "=> { :r :tzp ?s } ."
    ));
    assert!(r_pred(&c, "tzp"), "inSeconds with positive offset must fire; got {:?}", c);
}

// ---- reason_n3_terms_with_resolver with explicit base -----------------------

/// Line 291: `Some(b) => parser::parse_with_base(src, b)?` in
/// `reason_n3_terms_with_resolver`.
#[test]
fn reason_n3_terms_with_resolver_uses_base() {
    let c = reason_n3_terms_with_resolver(
        "@prefix : <http://ex/> . :a :b :c .",
        Some("http://base.example/"),
        None,
    )
    .expect("should parse with base");
    // 3 asserted facts; base parameter exercises line 291
    assert!(
        !c.facts.is_empty(),
        "resolver with base should return facts; got {:?}",
        c.facts
    );
}

// ---- incremental fallback serialisation -------------------------------------

/// `n3_write_term` lang-tagged literal (lines 2282-2288), special chars in
/// `n3_quote_into` (lines 2265-2269), and `is_empty` (lines 2724-2726).
#[test]
fn fallback_lang_tag_and_special_chars() {
    let rules = "@prefix : <http://ex/> . { :b :q :c } <= { :a :p :c } .";

    // Lang-tagged literal in base: n3_write_term Lit(_, _, Some(lang)) — lines 2282-2288
    let base_lang = vec![[
        Term::Iri("http://ex/s".into()),
        Term::Iri("http://ex/p".into()),
        Term::Lit(
            "bonjour".into(),
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString".into(),
            Some("fr".into()),
        ),
    ]];
    let g = MaterializedN3Graph::new(rules, &base_lang).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Fallback, "backward rule forces fallback");
    assert!(!g.is_empty(), "non-empty fallback closure; is_empty() = false (lines 2724-2726)");

    // Special chars in literal lexical form — n3_quote_into lines 2265-2269
    let base_esc = vec![[
        Term::Iri("http://ex/s".into()),
        Term::Iri("http://ex/p".into()),
        Term::Lit(
            "a\\b\"c\nd\re\tf".into(), // backslash, quote, newline, CR, tab
            "http://www.w3.org/2001/XMLSchema#string".into(),
            None,
        ),
    ]];
    let g2 = MaterializedN3Graph::new(rules, &base_esc).expect("rules parse with special chars");
    assert_eq!(g2.mode(), N3Mode::Fallback);
    assert!(!g2.is_empty());
}

/// `n3_write_term` blank-node (lines 2299-2302), Var (2303-2306),
/// and List (2307-2313) branches in the fallback serialisation path.
#[test]
fn fallback_blank_var_and_list_in_base() {
    let rules = "@prefix : <http://ex/> . { :b :q :c } <= { :a :p :c } .";

    // Blank node subject — lines 2299-2302
    let base_blank = vec![[
        Term::Blank("bnode1".into()),
        Term::Iri("http://ex/p".into()),
        Term::Iri("http://ex/o".into()),
    ]];
    let g = MaterializedN3Graph::new(rules, &base_blank).expect("rules parse with blank node");
    assert_eq!(g.mode(), N3Mode::Fallback, "backward rule forces fallback");
    assert!(!g.is_empty());

    // Variable subject: `n3_write_term(Var)` → `?v` — lines 2303-2306;
    // valid N3 (universally-quantified), re-parses without error.
    let base_var = vec![[
        Term::Var("v".into()),
        Term::Iri("http://ex/p".into()),
        Term::Iri("http://ex/o".into()),
    ]];
    let _gv = MaterializedN3Graph::new(rules, &base_var).expect("rules parse with var in base");

    // List subject: `n3_write_term(List)` → `( elem … )` — lines 2307-2313;
    // N3 list-in-subject position is valid syntax (expands to rdf:first/rest).
    let base_list = vec![[
        Term::List(vec![Term::Iri("http://ex/a".into()), Term::Iri("http://ex/b".into())]),
        Term::Iri("http://ex/p".into()),
        Term::Iri("http://ex/o".into()),
    ]];
    let _gl = MaterializedN3Graph::new(rules, &base_list)
        .expect("rules parse with list subject in base");
}
