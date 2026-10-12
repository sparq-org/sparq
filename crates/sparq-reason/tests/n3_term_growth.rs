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
    let small = ClosureLimits {
        max_term_bytes: 1 << 16,
        ..ClosureLimits::default()
    };
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
    let small = ClosureLimits {
        max_term_bytes: 1 << 12,
        ..ClosureLimits::default()
    };
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
    let tight = ClosureLimits {
        max_term_bytes: 1 << 12,
        ..ClosureLimits::default()
    };
    let same = reason_n3_terms_with_limits(&doc, None, None, NegationCycles::Reject, tight)
        .expect("closes within the limit");
    assert_eq!(same.facts.len(), closed.facts.len());
}
