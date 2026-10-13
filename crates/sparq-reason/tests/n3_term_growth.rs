//! GH #6771: a rule set whose derived TERMS grow (a string or list that doubles each
//! round) is refused by the forward closure's term-size limit with an `Err`, instead of
//! aborting the process when an allocation fails. The round and fact allowances cannot
//! catch it: the closure holds a few dozen facts after a few dozen rounds.
//!
//! The repro runs under the DEFAULT limits, so this binary is also meant to pass under a
//! bounded address space (`ulimit -v`).

use sparq_reason::n3::{
    reason_n3_terms, reason_n3_terms_with_limits, ClosureLimits, NegationCycles,
};

/// The issue's repro: each round concatenates the literal with itself.
const STRING_DOUBLING: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     :a :p \"x\".\n\
     { :a :p ?s. (?s ?s) string:concatenation ?t } => { :a :p ?t }.\n";

/// The same growth through a first-class list.
const LIST_DOUBLING: &str = "@prefix : <http://ex/>.\n\
     @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n\
     :a :p (1).\n\
     { :a :p ?l. (?l ?l) list:append ?m } => { :a :p ?m }.\n";

/// The same growth through a `string:format` that repeats its argument.
const FORMAT_DOUBLING: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     :a :p \"x\".\n\
     { :a :p ?s. (\"%s%s\" ?s ?s) string:format ?t } => { :a :p ?t }.\n";

/// The same growth through a `string:replace` whose replacement repeats every match.
const REPLACE_DOUBLING: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     :a :p \"x\".\n\
     { :a :p ?s. (?s \"x\" \"xx\") string:replace ?t } => { :a :p ?t }.\n";

fn assert_term_limit<T: std::fmt::Debug>(what: &str, r: Result<T, String>) {
    match r {
        Err(e) => {
            assert!(e.contains("forward closure limit exceeded"), "{what}: {e}");
            assert!(e.contains("max_term_bytes"), "{what}: {e}");
        }
        Ok(v) => panic!("{what}: a truncated closure came back as a result: {v:?}"),
    }
}

#[test]
fn string_doubling_is_refused_under_the_default_limits() {
    assert_term_limit(
        "string:concatenation doubling",
        reason_n3_terms(STRING_DOUBLING, None).map(|c| c.facts.len()),
    );
}

#[test]
fn every_growing_builtin_is_refused_under_a_small_term_limit() {
    let mut small = ClosureLimits::default();
    small.max_term_bytes = 1 << 16;
    for (what, doc) in [
        ("string:concatenation", STRING_DOUBLING),
        ("list:append", LIST_DOUBLING),
        ("string:format", FORMAT_DOUBLING),
        ("string:replace", REPLACE_DOUBLING),
    ] {
        assert_term_limit(
            what,
            reason_n3_terms_with_limits(doc, None, None, NegationCycles::Reject, small)
                .map(|c| c.facts.len()),
        );
    }
}

#[test]
fn a_nested_closure_that_grows_a_term_fails_the_run() {
    let doc = "@prefix : <http://ex/>.\n\
         @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
         @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
         :w :is { :a :p \"x\". { :a :p ?s. (?s ?s) string:concatenation ?t } => { :a :p ?t } }.\n\
         { :w :is ?f. ?f log:conclusion ?c } => { :w :closed ?c }.\n";
    let mut small = ClosureLimits::default();
    small.max_term_bytes = 1 << 12;
    assert_term_limit(
        "nested log:conclusion",
        reason_n3_terms_with_limits(doc, None, None, NegationCycles::Reject, small)
            .map(|c| c.facts.len()),
    );
}

#[test]
fn bounded_string_builtins_still_compute_their_values() {
    // Growth that stops (a guard on the length) closes under the defaults, and every
    // string builtin still computes its value.
    let doc = "@prefix : <http://ex/>.\n\
         @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
         @prefix math: <http://www.w3.org/2000/10/swap/math#>.\n\
         :a :p \"x\".\n\
         { :a :p ?s. ?s string:length ?n. ?n math:lessThan 1000. (?s ?s) string:concatenation ?t } => { :a :p ?t }.\n\
         { (\"ab\" \"cd\" 5 \"1E3\"^^<http://www.w3.org/2001/XMLSchema#double>) string:concatenation ?t } => { :r :concat ?t }.\n\
         { (\"%s-%d\" \"v\" 7) string:format ?t } => { :r :format ?t }.\n\
         { (\"banana\" \"a\" \"[$0]\") string:replace ?t } => { :r :replace ?t }.\n\
         { \"MiXed\" string:lowerCase ?t } => { :r :lower ?t }.\n\
         { \"MiXed\" string:upperCase ?t } => { :r :upper ?t }.\n\
         { \"a b/c\" string:encodeForUri ?t } => { :r :uri ?t }.\n\
         { ((1 2) (3)) list:append ?t } => { :r :append ?t }.\n";
    let doc = format!("@prefix list: <http://www.w3.org/2000/10/swap/list#>.\n{doc}");
    let closed = reason_n3_terms(&doc, None).expect("closes");
    let text = format!("{:?}", closed.facts);
    for want in [
        "\"abcd51000\"",
        "\"v-7\"",
        "\"b[a]n[a]n[a]\"",
        "\"mixed\"",
        "\"MIXED\"",
        "\"a%20b%2Fc\"",
    ] {
        assert!(text.contains(want), "missing {want} in {text}");
    }
    // The doubling ran until the guard: "x" doubled to 1024 characters.
    assert!(text.contains(&"x".repeat(1024)), "the guarded doubling stopped early");
    assert!(!text.contains(&"x".repeat(2048)));
    // A term limit that the guarded workload stays inside changes nothing.
    let mut tight = ClosureLimits::default();
    tight.max_term_bytes = 1 << 12;
    let same = reason_n3_terms_with_limits(&doc, None, None, NegationCycles::Reject, tight)
        .expect("closes within the limit");
    assert_eq!(same.facts.len(), closed.facts.len());
}

/// `limits` with `max_term_bytes` set (the struct is `#[non_exhaustive]`).
fn term_limit(bytes: usize) -> ClosureLimits {
    let mut l = ClosureLimits::default();
    l.max_term_bytes = bytes;
    l
}

fn close(doc: &str, limits: ClosureLimits) -> Result<usize, String> {
    reason_n3_terms_with_limits(doc, None, None, NegationCycles::Reject, limits)
        .map(|c| c.facts.len())
}

const PREFIXES: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n\
     @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
     @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>.\n";

/// Review finding 1: sizing a replacement must not build it. A 4 MiB input whose every
/// match expands 600 copies of itself would be 2.4 GiB of text.
#[test]
fn a_huge_replacement_is_refused_without_building_it() {
    let doc = format!(
        "{PREFIXES}:a :p \"{}\".\n\
         {{ :a :p ?s. (?s \"x+\" \"{}\") string:replace ?t }} => {{ :a :q ?t }}.\n",
        "x".repeat(4 << 20),
        "$0".repeat(600)
    );
    assert_term_limit("string:replace", close(&doc, ClosureLimits::default()));
}

/// Review finding 2: the argument list is sized before it is built. 128 references to
/// an 8 MiB binding would be a 1 GiB argument list.
#[test]
fn a_huge_argument_list_is_refused_before_it_is_built() {
    let refs = "?s ".repeat(128);
    let doc = format!(
        "{PREFIXES}:a :p \"{}\".\n\
         {{ :a :p ?s. ({refs}) string:concatenation ?t }} => {{ :a :q ?t }}.\n",
        "x".repeat(8 << 20)
    );
    assert_term_limit("128-reference argument list", close(&doc, ClosureLimits::default()));
}

/// Review finding 3: `list:append` over rdf:first/rest data lists is charged for the
/// members it copies, so a list that doubles through data-list cells is refused.
#[test]
fn data_list_append_doubling_is_refused() {
    let doc = format!(
        "{PREFIXES}:c rdf:first (\"x\"); rdf:rest rdf:nil.\n\
         {{ ?c rdf:first ?l. (?c ?c) list:append ?m }} => {{ [] rdf:first ?m; rdf:rest rdf:nil }}.\n"
    );
    assert_term_limit("data-list list:append", close(&doc, term_limit(1 << 16)));
}

/// Review finding 4: the formula `log:parsedAsN3` returns is sized before it is derived.
/// One 64 KiB namespace used in 128 triples expands to 24 MiB of IRIs. (The parse itself
/// is bounded by its input, not by `max_term_bytes`.)
#[test]
fn a_parse_that_expands_past_the_limit_is_refused() {
    let mut src = format!("@prefix p: <http://ex/{}#>. ", "a".repeat(64 << 10));
    for i in 0..128 {
        src.push_str(&format!("p:s{i} p:p p:o . "));
    }
    let doc = format!(
        "{PREFIXES}:a :src \"\"\"{src}\"\"\".\n\
         {{ :a :src ?s. ?s log:parsedAsN3 ?f }} => {{ :a :parsed ?f }}.\n"
    );
    assert_term_limit("log:parsedAsN3", close(&doc, ClosureLimits::default()));
    // A small parse still works under the default.
    let small = format!(
        "{PREFIXES}:a :src \"\"\"@prefix p: <http://ex/q#>. p:s p:p p:o .\"\"\".\n\
         {{ :a :src ?s. ?s log:parsedAsN3 ?f }} => {{ :a :parsed ?f }}.\n"
    );
    let closed = reason_n3_terms(&small, None).expect("closes");
    assert!(format!("{:?}", closed.facts).contains("http://ex/q#s"));
}

/// Review finding 5: a loose bound over the limit does not refuse a result that fits.
/// Case mapping and URI encoding of 6 MiB of ASCII are bounded at 3x (18 MiB, over the
/// 16 MiB default), but their results are 6 MiB.
#[test]
fn a_result_that_fits_is_not_refused_by_a_loose_bound() {
    let doc = format!(
        "{PREFIXES}:a :p \"{}\".\n\
         {{ :a :p ?s. ?s string:upperCase ?u }} => {{ :a :upper ?u }}.\n\
         {{ :a :p ?s. ?s string:lowerCase ?u }} => {{ :a :lower ?u }}.\n\
         {{ :a :p ?s. ?s string:encodeForUri ?u }} => {{ :a :uri ?u }}.\n",
        "x".repeat(6 << 20)
    );
    assert_eq!(close(&doc, ClosureLimits::default()), Ok(4));
}

/// `log:conclusion` returns a formula holding its whole closure: that formula is one
/// more term, refused past the limit even though each fact in it is small.
#[test]
fn a_large_concluded_formula_is_refused() {
    let mut inner = String::new();
    for i in 0..200 {
        inner.push_str(&format!(":s{i} :p :o{i}. "));
    }
    let doc = format!(
        "{PREFIXES}:w :is {{ {inner} }}.\n\
         {{ :w :is ?f. ?f log:conclusion ?c }} => {{ :w :closed ?c }}.\n"
    );
    assert!(close(&doc, ClosureLimits::default()).is_ok());
    assert_term_limit("log:conclusion", close(&doc, term_limit(4096)));
}

/// A rule conclusion that nests a bound term (no builtin at all) doubles it each round:
/// the conclusion's terms are sized before they are instantiated.
#[test]
fn conclusion_list_doubling_is_refused() {
    let doc = format!(
        "{PREFIXES}:a :p \"x\".\n\
         {{ :a :p ?s }} => {{ :a :p (?s ?s) }}.\n"
    );
    assert_term_limit("list conclusion", close(&doc, term_limit(1 << 16)));
    let formula = format!(
        "{PREFIXES}:a :p \"x\".\n\
         {{ :a :p ?s }} => {{ :a :p {{ ?s :q ?s }} }}.\n"
    );
    assert_term_limit("formula conclusion", close(&formula, term_limit(1 << 16)));
}

/// `log:collectAllIn` builds one copy of its template per solution: the collected list
/// is sized before it is built.
#[test]
fn a_large_collected_list_is_refused() {
    let mut doc = format!(
        "{PREFIXES}:a :big \"{}\".\n\
         {{ :a :big ?b. ((?x ?b) {{ ?x :n ?i }} ?all) log:collectAllIn _:t }} => {{ :a :all ?all }}.\n",
        "y".repeat(1024)
    );
    for i in 0..64 {
        doc.push_str(&format!(":e{i} :n {i}.\n"));
    }
    assert!(close(&doc, ClosureLimits::default()).is_ok());
    let e = close(&doc, term_limit(16 << 10)).expect_err("refused");
    assert!(e.contains("collectAllIn"), "{e}");
}

/// Review round 3, finding 1: every substitution is sized before it is built. A
/// collection template naming an 8 MiB binding 128 times would be 1 GiB.
#[test]
fn a_huge_collection_template_is_refused_before_it_is_built() {
    let refs = "?s ".repeat(128);
    let doc = format!(
        "{PREFIXES}:a :p \"{}\". :e :n 1.\n\
         {{ :a :p ?s. (({refs}) {{ ?x :n ?i }} ?all) log:collectAllIn _:t }} => {{ :a :all ?all }}.\n",
        "x".repeat(8 << 20)
    );
    assert_term_limit("collection template", close(&doc, ClosureLimits::default()));
}

/// Review round 3, finding 1: a list generator's members and a backward rule's
/// projection are substituted through the same sized primitive.
#[test]
fn list_generator_and_backward_projection_are_sized() {
    let refs = "?s ".repeat(128);
    let generator = format!(
        "{PREFIXES}:a :p \"{}\".\n\
         {{ :a :p ?s. ?m list:in (({refs})) }} => {{ :a :m ?m }}.\n",
        "x".repeat(8 << 20)
    );
    assert_term_limit("list generator", close(&generator, ClosureLimits::default()));
    let backward = format!(
        "{PREFIXES}:a :p \"{}\".\n\
         {{ :a :q ?l }} <= {{ :a :p ?s. ?l log:equalTo ({refs}) }}.\n\
         {{ :a :q ?l }} => {{ :a :r ?l }}.\n",
        "x".repeat(8 << 20)
    );
    assert_term_limit("backward projection", close(&backward, ClosureLimits::default()));
}

/// Review round 3, finding 2: the parsed formula is sized with every term in it, a
/// literal object included, so one with a 64 KiB literal is refused under a 4 KiB limit.
#[test]
fn a_parsed_literal_object_is_charged() {
    let src = format!("<http://ex/s> <http://ex/p> \"{}\" .", "z".repeat(64 << 10));
    let doc = format!(
        "{PREFIXES}:a :src \"\"\"{src}\"\"\".\n\
         {{ :a :src ?s. ?s log:parsedAsN3 ?f }} => {{ :a :parsed ?f }}.\n"
    );
    assert_term_limit("parsed literal object", close(&doc, term_limit(4096)));
    // Numeric list members and list nodes are counted too.
    let list = format!("<http://ex/s> <http://ex/p> ({}) .", "1 ".repeat(4096));
    let doc = format!(
        "{PREFIXES}:a :src \"\"\"{list}\"\"\".\n\
         {{ :a :src ?s. ?s log:parsedAsN3 ?f }} => {{ :a :parsed ?f }}.\n"
    );
    assert_term_limit("parsed numeric list", close(&doc, term_limit(16 << 10)));
}

/// Review round 3, finding 3: each term of the parsed formula is counted once. A 64 KiB
/// namespace used in 60 three-IRI triples is about 11 MiB of formula and fits under the
/// 16 MiB default.
#[test]
fn a_parse_that_fits_is_not_double_charged() {
    let mut src = format!("@prefix p: <http://ex/{}#>. ", "a".repeat(64 << 10));
    for i in 0..60 {
        src.push_str(&format!("p:s{i} p:p p:o . "));
    }
    let doc = format!(
        "{PREFIXES}:a :src \"\"\"{src}\"\"\".\n\
         {{ :a :src ?s. ?s log:parsedAsN3 ?f }} => {{ :a :parsed ?f }}.\n"
    );
    assert_eq!(close(&doc, ClosureLimits::default()), Ok(2));
}

/// Review round 4, finding 3: `log:dtlit` builds its literal by an early return; the
/// literal is sized from the borrowed lexical form and datatype IRI before it is copied.
/// Each is 9 MiB, under the 16 MiB default; the literal would be 18 MiB.
#[test]
fn a_dtlit_over_the_limit_is_refused() {
    let doc = format!(
        "{PREFIXES}:a :p \"{}\". :a :d <http://ex/{}>.\n\
         {{ :a :p ?s. :a :d ?d. (?s ?d) log:dtlit ?t }} => {{ :a :lit ?t }}.\n",
        "x".repeat(9 << 20),
        "y".repeat(9 << 20)
    );
    assert_term_limit("log:dtlit", close(&doc, ClosureLimits::default()));
    let small = format!(
        "{PREFIXES}:a :p \"7\". :a :d <http://www.w3.org/2001/XMLSchema#integer>.\n\
         {{ :a :p ?s. :a :d ?d. (?s ?d) log:dtlit ?t }} => {{ :a :lit ?t }}.\n"
    );
    assert_eq!(close(&small, ClosureLimits::default()), Ok(3));
}

/// GH #6901: the bytes of a literal with lexical form `lex` and datatype `dt`, as
/// `max_term_bytes` counts them (string bytes plus one node).
fn literal_bytes(lex: &str, dt: &str) -> usize {
    std::mem::size_of::<sparq_reason::n3::Term>() + lex.len() + dt.len()
}

const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const MATH: &str = "@prefix math: <http://www.w3.org/2000/10/swap/math#>.\n";

/// Each `(rule, result)`: `doc` closes to `facts` facts under a limit of exactly the
/// result's size, and is refused one byte below it.
fn assert_exact_limit(facts: &str, cases: &[(&str, &str, usize)], want: usize) {
    for &(what, rule, size) in cases {
        let doc = format!("{PREFIXES}{MATH}{facts}{rule}\n");
        assert_eq!(close(&doc, term_limit(size)), Ok(want), "{what}: at its size ({size})");
        assert_term_limit(what, close(&doc, term_limit(size - 1)));
    }
}

/// GH #6901: an exact decimal result is sized from its value, scale included: accepted
/// under a limit of exactly its size and refused one byte below, forward and reversed.
#[test]
fn a_decimal_result_is_sized_from_its_scale() {
    let x = format!("0.{}1", "0".repeat(1000));
    let size = literal_bytes(&format!("-{x}"), XSD_DECIMAL);
    assert_exact_limit(
        &format!(":a :p {x}.\n"),
        &[
            ("math:negation", "{ :a :p ?x. ?x math:negation ?y } => { :a :q ?y }.", size),
            ("math:negation reversed", "{ :a :p ?x. ?y math:negation ?x } => { :a :q ?y }.", size),
        ],
        2,
    );
}

/// GH #6901: an `f64` result is sized from the text it renders as: an `xsd:double`
/// forward and reversed, and an `xsd:decimal` from a power past the exact tower.
#[test]
fn a_floating_result_is_sized_from_its_rendering() {
    let asin = format!("{:e}", 0.5f64.asin());
    let double = literal_bytes(&asin, XSD_DOUBLE);
    // 10^300 is past the exact tower, so it is computed as an f64 and printed in full.
    let power = literal_bytes(&format!("{}", 10f64.powf(300.0)), XSD_DECIMAL);
    assert_exact_limit(
        ":a :p 0.5.\n",
        &[
            ("math:asin", "{ :a :p ?x. ?x math:asin ?y } => { :a :q ?y }.", double),
            ("math:sin reversed", "{ :a :p ?x. ?y math:sin ?x } => { :a :q ?y }.", double),
            ("math:exponentiation", "{ :a :p ?x. (10 300) math:exponentiation ?y } => { :a :q ?y }.", power),
        ],
        2,
    );
}

/// GH #6901: `string:concatenation` sizes a numeric piece by the canonical text it
/// writes (a double of five lexical bytes writes 301 digits).
#[test]
fn concatenated_numeric_pieces_are_sized_from_their_values() {
    let size = literal_bytes(&format!("{}", 1e300f64), XSD_STRING);
    assert_exact_limit(
        ":a :p \"1E300\"^^<http://www.w3.org/2001/XMLSchema#double>.\n",
        &[("string:concatenation", "{ :a :p ?x. (?x) string:concatenation ?y } => { :a :q ?y }.", size)],
        2,
    );
}

/// GH #6901 (review of #6833): `log:conjunction` of no formulae returns the literal
/// `true`, which is sized as that literal.
#[test]
fn an_empty_conjunction_is_sized_as_the_literal_it_returns() {
    let size = literal_bytes("true", XSD_BOOLEAN);
    assert_exact_limit(
        "",
        &[("log:conjunction", "{ () log:conjunction ?f } => { :a :q ?f }.", size)],
        1,
    );
}
