//! N3 builtin correctness — reverse/invertible modes, comparison operators, and the
//! error/edge paths (arg-count + type mismatch).
//!
//! 🤖 SPARQ agent — sq-qcnn test-quality slice.
//!
//! Differential against cwm/EYE builtin semantics. Each case is a self-contained N3 document
//! whose forward rule's premise uses ONE builtin; the conclusion fires iff the builtin binds /
//! holds. We assert the EXACT entailed marker triple is present when it should be, and ABSENT
//! when the builtin must FAIL the premise (a type mismatch, a domain error, an arg-count
//! mismatch, or an invalid regex). This pins the "fail-closed" boundary the builtins rely on.

use sparq_core::dict::Dict;
use sparq_reason::reason_n3;
use std::collections::HashSet;

/// Closure of `src` as canonical (s, p, o) strings. Subjects/predicates/objects are rendered
/// in the `Dict::term` (oxrdf Display / N-Triples) form: IRIs as `<…>`, typed literals as
/// `"lex"^^<dt>`, plain xsd:string literals as `"lex"`.
fn closure(src: &str) -> HashSet<(String, String, String)> {
    let mut d = Dict::new();
    reason_n3(&mut d, src)
        .expect("reasoning failed")
        .iter()
        .map(|[s, p, o]| {
            (
                d.term(*s).to_string(),
                d.term(*p).to_string(),
                d.term(*o).to_string(),
            )
        })
        .collect()
}

/// IRI as it renders from `Dict::term`: `<iri>`.
fn ir(s: &str) -> String {
    format!("<{}>", s)
}
/// Predicate at `http://ex/<local>` in rendered form.
fn p_ex(local: &str) -> String {
    format!("<http://ex/{}>", local)
}
/// xsd:string-typed literal as it renders: a plain `"lex"` (xsd:string is the implicit type).
fn str_lit(v: &str) -> String {
    format!("\"{}\"", v)
}
/// A typed literal as it renders: `"lex"^^<dt>`.
fn typed_lit(v: &str, dt: &str) -> String {
    format!("\"{}\"^^<{}>", v, dt)
}
const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DEC: &str = "http://www.w3.org/2001/XMLSchema#decimal";

/// Does the closure contain a triple whose subject is `:r`, predicate `:<local>`, object `obj`?
fn r_has(c: &HashSet<(String, String, String)>, local: &str, obj: &str) -> bool {
    c.contains(&(ir("http://ex/r"), p_ex(local), obj.to_string()))
}
/// Does ANY triple have subject `:r` and predicate `:<local>` (regardless of object)?
fn r_pred(c: &HashSet<(String, String, String)>, local: &str) -> bool {
    c.iter()
        .any(|(s, p, _)| *s == ir("http://ex/r") && *p == p_ex(local))
}

const PRE: &str = "@prefix : <http://ex/> .\n\
    @prefix math: <http://www.w3.org/2000/10/swap/math#> .\n\
    @prefix string: <http://www.w3.org/2000/10/swap/string#> .\n\
    @prefix list: <http://www.w3.org/2000/10/swap/list#> .\n\
    @prefix log: <http://www.w3.org/2000/10/swap/log#> .\n\
    @prefix time: <http://www.w3.org/2000/10/swap/time#> .\n";

/// Build a document: prefixes + the given body, run it, return the closure.
fn run(body: &str) -> HashSet<(String, String, String)> {
    closure(&format!("{}{}", PRE, body))
}

/// Extract the OBJECT string of the first (:r :<local> o) triple, if any.
fn r_obj<'a>(c: &'a HashSet<(String, String, String)>, local: &str) -> Option<&'a str> {
    c.iter()
        .find(|(s, p, _)| *s == ir("http://ex/r") && *p == p_ex(local))
        .map(|(_, _, o)| o.as_str())
}

// ---------- REVERSE / INVERTIBLE modes ----------

#[test]
fn math_negation_reverse_binds_the_subject() {
    // `?x math:negation 3` solves ?x = -3 (cwm's one reverse mode the suites rely on).
    let c = run("{ ?x math:negation 3 } => { :r :is ?x } .");
    assert!(
        r_has(&c, "is", &typed_lit("-3", XSD_INT)),
        "negation reverse: -3; got {:?}",
        c
    );
}

#[test]
fn math_negation_reverse_on_decimal() {
    // Reverse negation must keep the numeric tower: ?x math:negation 2.5 ⊢ ?x = -2.5 (decimal).
    let c = run("{ ?x math:negation 2.5 } => { :r :is ?x } .");
    assert!(
        r_has(&c, "is", &typed_lit("-2.5", XSD_DEC)),
        "negation reverse decimal; got {:?}",
        c
    );
}

#[test]
fn math_sin_reverse_solves_arcsine() {
    // `?y math:sin 0` solves y = asin(0) = 0 (cwm trig.n3 test4). xsd:double output.
    let c = run("{ ?y math:sin 0 } => { :r :asin0 ?y } .");
    let o = r_obj(&c, "asin0").expect("sin reverse should bind a value");
    // Strip the `"…"^^<xsd:double>` wrapper and check the numeric value ≈ 0.
    let num = o.trim_start_matches('"').split('"').next().unwrap_or(o);
    assert!(
        num.parse::<f64>().map(|v| v.abs() < 1e-12).unwrap_or(false),
        "sin reverse should bind y≈0; got {:?}",
        c
    );
}

#[test]
fn time_in_seconds_reverse_formats_epoch_back_to_datetime() {
    // `?t time:inSeconds 0` formats epoch 0 back to the UTC dateTime 1970-01-01T00:00:00Z.
    let c = run("{ ?t time:inSeconds 0 } => { :r :at ?t } .");
    let o = r_obj(&c, "at").expect("inSeconds reverse should bind a value");
    assert!(
        o.contains("1970-01-01T00:00:00"),
        "inSeconds reverse should format epoch 0 to 1970-01-01T00:00:00Z; got {:?}",
        c
    );
}

// ---------- COMPARISON operators (eval_builtin numeric + string branches) ----------

#[test]
fn numeric_comparisons_hold_and_fail_correctly() {
    // math:lessThan, math:notGreaterThan, math:notLessThan, math:equalTo, math:notEqualTo.
    let mp = "@prefix : <http://ex/> .\n@prefix math: <http://www.w3.org/2000/10/swap/math#> .\n";
    let c = closure(&format!(
        "{}{}",
        mp,
        ":x :v 3 .\n\
         { :x :v ?n . ?n math:lessThan 5 } => { :x :lt5 true } .\n\
         { :x :v ?n . ?n math:notGreaterThan 3 } => { :x :ng3 true } .\n\
         { :x :v ?n . ?n math:notLessThan 3 } => { :x :nl3 true } .\n\
         { :x :v ?n . ?n math:equalTo 3 } => { :x :eq3 true } .\n\
         { :x :v ?n . ?n math:notEqualTo 4 } => { :x :ne4 true } .\n\
         { :x :v ?n . ?n math:lessThan 1 } => { :x :lt1 true } ."
    ));
    let yes = |local: &str| {
        c.iter()
            .any(|(s, p, _)| *s == ir("http://ex/x") && *p == p_ex(local))
    };
    assert!(yes("lt5"), "3 < 5 holds; got {:?}", c);
    assert!(yes("ng3"), "3 ≤ 3 holds");
    assert!(yes("nl3"), "3 ≥ 3 holds");
    assert!(yes("eq3"), "3 == 3 holds");
    assert!(yes("ne4"), "3 != 4 holds");
    assert!(
        !yes("lt1"),
        "3 < 1 must NOT hold — premise fails, conclusion absent"
    );
}

#[test]
fn string_ordering_and_prefix_comparisons() {
    // string:greaterThan / lessThan / notGreaterThan / notLessThan / endsWith / startsWith.
    let sp =
        "@prefix : <http://ex/> .\n@prefix string: <http://www.w3.org/2000/10/swap/string#> .\n";
    let c = closure(&format!(
        "{}{}",
        sp,
        "{ \"banana\" string:greaterThan \"apple\" } => { :r :gt true } .\n\
         { \"apple\" string:lessThan \"banana\" } => { :r :lt true } .\n\
         { \"apple\" string:notGreaterThan \"apple\" } => { :r :ng true } .\n\
         { \"apple\" string:notLessThan \"apple\" } => { :r :nl true } .\n\
         { \"filename.txt\" string:endsWith \".txt\" } => { :r :ends true } .\n\
         { \"http://x\" string:startsWith \"http\" } => { :r :starts true } .\n\
         { \"apple\" string:greaterThan \"banana\" } => { :r :badgt true } ."
    ));
    for local in ["gt", "lt", "ng", "nl", "ends", "starts"] {
        assert!(
            r_pred(&c, local),
            "string comparison {} should hold; got {:?}",
            local,
            c
        );
    }
    assert!(
        !r_pred(&c, "badgt"),
        "\"apple\" > \"banana\" must NOT hold; got {:?}",
        c
    );
}

// ---------- ERROR / EDGE handling (fail the premise, do not panic) ----------

#[test]
fn math_builtin_on_a_non_number_fails_the_premise() {
    // math:sum over a non-numeric argument must FAIL the premise — no conclusion, no panic.
    let c = run("{ ( \"hello\" 2 ) math:sum ?x } => { :r :sum ?x } .");
    assert!(
        !r_pred(&c, "sum"),
        "math:sum on a non-number must fail closed; got {:?}",
        c
    );
}

#[test]
fn string_concatenation_coerces_iri_but_fails_on_a_formula_member() {
    // cwm coerces an IRI member to its text, so this SUCCEEDS with the IRI string.
    let c = run("{ ( :anIri ) string:concatenation ?x } => { :r :cat ?x } .");
    assert!(
        r_has(&c, "cat", &str_lit("http://ex/anIri")),
        "string:concatenation coerces an IRI member to its text (cwm); got {:?}",
        c
    );
    // A formula member is NOT coercible — must fail closed.
    let c2 = run("{ ( {:a :b :c} ) string:concatenation ?x } => { :r :bad ?x } .");
    assert!(
        !r_pred(&c2, "bad"),
        "string:concatenation of a formula member fails closed; got {:?}",
        c2
    );
}

#[test]
fn invalid_regex_in_string_matches_fails_the_premise() {
    // string:matches with an unparseable regex must fail the premise (no panic).
    let c = run("{ \"abc\" string:matches \"[unclosed\" } => { :r :matched true } .");
    assert!(
        !r_pred(&c, "matched"),
        "an invalid regex must fail string:matches closed; got {:?}",
        c
    );
    // A VALID regex must still match (positive control).
    let c2 = run("{ \"abc123\" string:matches \"[a-z]+[0-9]+\" } => { :r :ok true } .");
    assert!(r_pred(&c2, "ok"), "a valid regex matches; got {:?}", c2);
}

#[test]
fn string_format_arg_count_mismatch_fails_the_premise() {
    // string:format with too FEW args for the directives must fail (EYE throws; we fail closed).
    let c = run("{ ( \"%s and %s\" \"only-one\" ) string:format ?x } => { :r :fmt ?x } .");
    assert!(
        !r_pred(&c, "fmt"),
        "format with a missing arg fails closed; got {:?}",
        c
    );
    // Exact arg count → success with the substituted string.
    let c2 = run("{ ( \"%s=%d\" \"x\" 7 ) string:format ?x } => { :r :ok ?x } .");
    assert!(
        r_has(&c2, "ok", &str_lit("x=7")),
        "format %s=%d → x=7; got {:?}",
        c2
    );
}

#[test]
fn unary_math_on_a_list_argument_fails_the_premise() {
    // A unary math builtin takes a DIRECT value, never a ( … ) list: `(1) math:rounded ?x`
    // must fail (cwm/EYE assert this via :FAILURE rules).
    let c = run("{ ( 1 ) math:rounded ?x } => { :r :rounded ?x } .");
    assert!(
        !r_pred(&c, "rounded"),
        "unary math on a list must fail closed; got {:?}",
        c
    );
    // The direct form succeeds: 1.4 rounds to 1 (round-half-up). Integer input → integer 1.
    let c2 = run("{ 1.4 math:rounded ?x } => { :r :ok ?x } .");
    assert!(
        r_pred(&c2, "ok"),
        "1.4 math:rounded must bind a value; got {:?}",
        c2
    );
    // decimal-in → the cwm reference keeps the decimal type: 1.4 rounds to "1.0".
    assert!(
        r_has(&c2, "ok", &typed_lit("1.0", XSD_DEC)),
        "1.4 math:rounded → 1.0 (decimal-in keeps decimal type); got {:?}",
        c2
    );
}

#[test]
fn math_quotient_exact_division_by_zero_fails() {
    // Exact-arithmetic division by zero fails the premise (no panic); a double 1.0/0.0 would
    // propagate IEEE INF, but the integer path fails closed.
    let c = run("{ ( 1 0 ) math:quotient ?x } => { :r :q ?x } .");
    assert!(
        !r_pred(&c, "q"),
        "integer division by zero fails closed; got {:?}",
        c
    );
}

#[test]
fn math_member_count_on_a_list_and_a_formula() {
    // math:memberCount of a ( … ) list is its length; of a quoted formula, its DISTINCT triple
    // count (cwm). Both via the functional dispatch.
    let c = run("{ ( :a :b :c ) math:memberCount ?n } => { :r :listlen ?n } .");
    assert!(
        r_has(&c, "listlen", &typed_lit("3", XSD_INT)),
        "list memberCount = 3; got {:?}",
        c
    );
    let c2 = run("{ {:a :p :b . :c :p :d} math:memberCount ?n } => { :r :flen ?n } .");
    assert!(
        r_has(&c2, "flen", &typed_lit("2", XSD_INT)),
        "formula memberCount = 2 distinct triples; got {:?}",
        c2
    );
}
