//! GH #6771: the terms the N3 text engine derives are sized BEFORE they are built (numeric
//! lexical forms excepted: they are rendered, then checked, GH #6901). This
//! binary counts live heap bytes with a counting global allocator and drives each
//! builtin's result path (and the other derived-term paths: substitution, aggregation,
//! conclusions) past a small `max_term_bytes`: the call must fail with the closure-limit
//! error naming that path, and its peak heap must stay within half the refused term's
//! size of a control run that holds the same inputs but derives nothing large. A path
//! that built the term and checked it afterwards would show the whole term in its peak.
//! Each refused term is many times the inputs the evaluation holds copies of, so those
//! copies cannot hide it; the exceptions say how they are checked instead ([`Check`]).
//!
//! The peak is taken from the moment rule evaluation starts, not from the start of the
//! call: parsing the document holds several copies of each large input, and that peak
//! would hide a term built during evaluation. Every rule's first premise reads a document
//! through the resolver, and the first such read marks the start.
//!
//! One `#[test]` drives every case in turn, so no other test thread allocates while a
//! case is measured.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use sparq_reason::n3::{reason_n3_terms_with_limits, ClosureLimits, NegationCycles};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Bytes allocated (counting a reallocation's growth) since the mark.
static TOTAL: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards the caller's layout to the system allocator unchanged.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            TOTAL.fetch_add(layout.size(), Ordering::Relaxed);
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by `alloc` above with this `layout`.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` and `layout` come from this allocator; `new_size` is the caller's.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            TOTAL.fetch_add(new_size.saturating_sub(layout.size()), Ordering::Relaxed);
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            let now = LIVE.fetch_add(new_size, Ordering::Relaxed) + new_size;
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// The term-size limit every case runs under.
const LIMIT: usize = 64 << 10;
/// The size of each large input: under the limit, while every refused term is at least
/// 1.1 times the limit.
const S: usize = 40 << 10;
/// A small input that a rule names [`K`] times: the refused term is [`K`] times it, far
/// more than the copies of it the evaluation holds.
const SMALL: usize = 8 << 10;
/// See [`SMALL`].
const K: usize = 16;

const PREFIXES: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n\
     @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
     @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>.\n\
     @prefix xsd: <http://www.w3.org/2001/XMLSchema#>.\n\
     @prefix time: <http://www.w3.org/2000/10/swap/time#>.\n";

/// The live heap when rule evaluation started, set by the first resolver read.
static MARK: AtomicUsize = AtomicUsize::new(usize::MAX);

/// The document every rule's first premise reads, marking the start of evaluation.
const MARK_PREMISE: &str = "<http://ex/mark> log:content ?mark. ";

/// `rule` with [`MARK_PREMISE`] as its first premise.
fn marked(rule: &str) -> String {
    let body = rule.strip_prefix("{ ").expect("a rule starts with `{ `");
    format!("{{ {MARK_PREMISE}{body}")
}

/// A document the resolver supplies as [`BIG`] bytes of text (`log:content`).
const BIG_DOC: &str = "http://ex/big";
/// A document whose every read restarts the count of bytes allocated
/// ([`Check::AfterLastRead`]).
const LATE_DOC: &str = "http://ex/late";
/// The size of [`BIG_DOC`]'s text, over [`LIMIT`].
const BIG: usize = 70 << 10;

/// What one run allocated during rule evaluation (from the first resolver read).
struct Measured {
    /// The peak heap above the live heap at the mark.
    peak: usize,
    /// The bytes allocated after the mark, or after the last read of [`LATE_DOC`].
    total: usize,
}

/// The closure of `doc` under [`LIMIT`], and what it allocated from the first resolver
/// read (the start of rule evaluation).
fn measured(doc: &str) -> (Result<usize, String>, Measured) {
    let mut limits = ClosureLimits::default();
    limits.max_term_bytes = LIMIT;
    MARK.store(usize::MAX, Ordering::Relaxed);
    let mark = |iri: &str| -> Option<String> {
        if MARK.load(Ordering::Relaxed) == usize::MAX {
            let live = LIVE.load(Ordering::Relaxed);
            MARK.store(live, Ordering::Relaxed);
            PEAK.store(live, Ordering::Relaxed);
            TOTAL.store(0, Ordering::Relaxed);
        }
        if iri == LATE_DOC {
            TOTAL.store(0, Ordering::Relaxed);
        }
        Some(if iri == BIG_DOC {
            "z".repeat(BIG)
        } else {
            String::new()
        })
    };
    let r = reason_n3_terms_with_limits(doc, None, Some(&mark), NegationCycles::Reject, limits)
        .map(|c| c.facts.len());
    let base = MARK.load(Ordering::Relaxed);
    assert_ne!(base, usize::MAX, "no rule read the mark document:\n{doc}");
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(base);
    (
        r,
        Measured {
            peak,
            total: TOTAL.load(Ordering::Relaxed),
        },
    )
}

/// How a case shows that the refused term was not built before it was sized.
#[derive(Clone, Copy)]
enum Check {
    /// The peak heap stays within half the refused term of the control's.
    Peak,
    /// The bytes allocated stay within half the refused term of the control's: for a term
    /// at most three times an input the evaluation already holds copies of, whose peak
    /// cannot tell a built term from those copies.
    Allocated,
    /// The bytes allocated after the last read of [`LATE_DOC`] stay under half the refused
    /// term: for a term built from work that copies its inputs many times over (a nested
    /// closure), which no control can match. A rule inside the closed formula reads
    /// [`LATE_DOC`], so the count covers only the end of the closure and what follows it.
    AfterLastRead,
    /// Only the refusal: `log:content`'s literal takes the resolver's string without
    /// copying it, so building it first allocates nothing to see.
    RefusalOnly,
}

struct Case {
    what: &'static str,
    facts: String,
    /// The rule that derives the oversized term.
    rule: String,
    /// The same premise and builtin input, deriving nothing large.
    control: String,
    /// The size of the term that is refused.
    refused: usize,
    /// What the refusal must name: the builtin's result, the collected list, or a term
    /// built by substitution.
    names: &'static str,
    /// How the allocation order is checked.
    check: Check,
}

fn case(
    what: &'static str,
    facts: String,
    rule: &str,
    control: &str,
    refused: usize,
    names: &'static str,
) -> Case {
    Case {
        what,
        facts,
        rule: rule.into(),
        control: control.into(),
        refused,
        names,
        check: Check::Peak,
    }
}

/// `c`, checked with `check`.
fn checked(check: Check, c: Case) -> Case {
    Case { check, ..c }
}

/// A refusal of a term built by substituting a rule's bindings.
const SUBSTITUTION: &str = "a term built under a rule's bindings";

/// A control that derives nothing.
const NOTHING: &str = "{ :no :such :fact } => { :a :ok true }.";

/// `n` copies of `item`, space-separated.
fn times(item: &str, n: usize) -> String {
    vec![item; n].join(" ")
}

fn cases() -> Vec<Case> {
    let x = "x".repeat(S);
    let y = "y".repeat(BIG);
    let ok = "{ :a :p ?s } => { :a :ok true }.";
    // 300 doubles of 5 lexical bytes each: an argument list well under the limit, whose
    // members each print as 301 digits.
    let doubles = times("\"1E300\"^^xsd:double", 300);
    let small = format!(":a :p \"{}\".\n", "x".repeat(SMALL));
    let refs = times("?s", K);
    vec![
        // Each builtin's own result, from arguments that fit.
        case(
            "string:concatenation",
            String::new(),
            &format!("{{ ({doubles}) string:concatenation ?t }} => {{ :a :out ?t }}."),
            &format!("{{ ({doubles}) list:length ?n }} => {{ :a :ok true }}."),
            300 * 301,
            "the result of the builtin Concat",
        ),
        case(
            "string:format",
            String::new(),
            &format!(
                "{{ (\"{}\" {doubles}) string:format ?t }} => {{ :a :out ?t }}.",
                "%f".repeat(300)
            ),
            &format!(
                "{{ (\"{}\" {doubles}) list:length ?n }} => {{ :a :ok true }}.",
                "%f".repeat(300)
            ),
            300 * 308,
            "the result of the builtin Format",
        ),
        case(
            "string:replace",
            small.clone(),
            &format!(
                "{{ :a :p ?s. (?s \"x\" \"{}\") string:replace ?t }} => {{ :a :out ?t }}.",
                "x".repeat(K)
            ),
            "{ :a :p ?s. (?s \"x\" \"y\") list:length ?n } => { :a :ok true }.",
            K * SMALL,
            "the result of the builtin Replace",
        ),
        checked(
            Check::Allocated,
            case(
                "string:encodeForUri",
                String::new(),
                &format!(
                    "{{ \"{}\" string:encodeForUri ?t }} => {{ :a :out ?t }}.",
                    " ".repeat(S)
                ),
                &format!(
                    "{{ \"{}\" string:length ?n }} => {{ :a :ok true }}.",
                    " ".repeat(S)
                ),
                3 * S,
                "the result of the builtin EncodeForUri",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "string:encodeForURI",
                String::new(),
                &format!(
                    "{{ \"{}\" string:encodeForURI ?t }} => {{ :a :out ?t }}.",
                    " ".repeat(S)
                ),
                &format!(
                    "{{ \"{}\" string:length ?n }} => {{ :a :ok true }}.",
                    " ".repeat(S)
                ),
                3 * S,
                "the result of the builtin EncodeForUriCwm",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "string:encodeForFragID",
                String::new(),
                &format!(
                    "{{ \"{}\" string:encodeForFragID ?t }} => {{ :a :out ?t }}.",
                    " ".repeat(S)
                ),
                &format!(
                    "{{ \"{}\" string:length ?n }} => {{ :a :ok true }}.",
                    " ".repeat(S)
                ),
                3 * S,
                "the result of the builtin EncodeForFragId",
            ),
        ),
        // U+0390 (2 bytes) upper-cases to three characters (6 bytes).
        case(
            "string:upperCase",
            String::new(),
            &format!(
                "{{ \"{}\" string:upperCase ?t }} => {{ :a :out ?t }}.",
                "\u{390}".repeat(S / 2)
            ),
            &format!(
                "{{ \"{}\" string:length ?n }} => {{ :a :ok true }}.",
                "\u{390}".repeat(S / 2)
            ),
            3 * S,
            "the result of the builtin UpperCase",
        ),
        // U+0130 (2 bytes) lower-cases to two characters (3 bytes).
        checked(
            Check::Allocated,
            case(
                "string:lowerCase",
                String::new(),
                &format!(
                    "{{ \"{}\" string:lowerCase ?t }} => {{ :a :out ?t }}.",
                    "\u{130}".repeat(24 << 10)
                ),
                &format!(
                    "{{ \"{}\" string:length ?n }} => {{ :a :ok true }}.",
                    "\u{130}".repeat(24 << 10)
                ),
                72 << 10,
                "the result of the builtin LowerCase",
            ),
        ),
        case(
            "log:dtlit",
            String::new(),
            &format!(
                "{{ (\"{x}\" <http://ex/{}>) log:dtlit ?t }} => {{ :a :out ?t }}.",
                "y".repeat(S)
            ),
            NOTHING,
            2 * S,
            "the result of the builtin Dtlit",
        ),
        case(
            // A data list: the argument list holds two list heads; append copies members.
            "list:append",
            format!(":c rdf:first \"{x}\"; rdf:rest rdf:nil.\n"),
            "{ (:c :c) list:append ?t } => { :a :out ?t }.",
            "{ (:c :c) list:length ?n } => { :a :ok true }.",
            2 * S,
            "the result of the builtin Append",
        ),
        case(
            // Four solutions of 4 KiB, each collected as a list naming it eight times: every
            // item fits, the list of them does not.
            "log:collectAllIn",
            (0..4)
                .map(|i| format!(":a :c \"{}{i}\".\n", "x".repeat(SMALL / 2)))
                .collect(),
            &format!(
                "{{ (({}) {{ :a :c ?s }} ?all) log:collectAllIn _:t }} => {{ :a :out ?all }}.",
                times("?s", 8)
            ),
            "{ :a :c ?s } => { :a :ok true }.",
            4 * 8 * (SMALL / 2),
            "the list log:collectAllIn collects",
        ),
        // Substitution: a builtin's argument list, a list generator's list, and a
        // conclusion. (The results of `log:langlit` and `log:conjunction` are never more
        // than a constant larger than their argument lists, so it is the argument list
        // that is refused.)
        case(
            "string:concatenation arguments",
            small.clone(),
            &format!("{{ :a :p ?s. ({refs}) string:concatenation ?t }} => {{ :a :out ?t }}."),
            ok,
            K * SMALL,
            SUBSTITUTION,
        ),
        case(
            "log:conjunction arguments",
            format!(":a :f {{ :s :p \"{}\" }}.\n", "x".repeat(SMALL)),
            &format!(
                "{{ :a :f ?f. ({}) log:conjunction ?t }} => {{ :a :out ?t }}.",
                times("?f", K)
            ),
            "{ :a :f ?f } => { :a :ok true }.",
            K * SMALL,
            SUBSTITUTION,
        ),
        case(
            "list:in",
            small.clone(),
            &format!("{{ :a :p ?s. ?m list:in (({refs})) }} => {{ :a :out ?m }}."),
            ok,
            K * SMALL,
            SUBSTITUTION,
        ),
        case(
            "conclusion",
            small.clone(),
            &format!("{{ :a :p ?s }} => {{ :a :out ({refs}) }}."),
            ok,
            K * SMALL,
            SUBSTITUTION,
        ),
        // Results that copy, or are a part of, an input over the limit. The input is
        // the document's own (the parse is not sized) and reaches the builtin by a
        // variable (a copy, not sized): the result is sized itself.
        checked(
            Check::Allocated,
            case(
                "log:uri",
                format!(":a :i <http://ex/{}>.\n", y),
                "{ :a :i ?i. ?i log:uri ?u } => { :a :out ?u }.",
                "{ :a :i ?i } => { :a :ok true }.",
                BIG,
                "the result of the builtin LogUri",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "log:uri reverse",
                format!(":a :s \"{}\".\n", y),
                "{ :a :s ?s. ?i log:uri ?s } => { :a :out ?i }.",
                "{ :a :s ?s } => { :a :ok true }.",
                BIG,
                "the result of the builtin LogUri",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "log:dtlit reverse",
                format!(":a :l \"{}\"^^<http://ex/d>.\n", y),
                "{ :a :l ?l. (?x ?d) log:dtlit ?l } => { :a :out ?x }.",
                "{ :a :l ?l } => { :a :ok true }.",
                BIG,
                "the result of the builtin Dtlit",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "list:member",
                format!(":a :l (\"{}\").\n", y),
                "{ :a :l ?l. ?l list:member ?m } => { :a :out ?m }.",
                "{ :a :l ?l. ?l list:length ?n } => { :a :ok true }.",
                BIG,
                "the result of the builtin Member",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "list:iterate",
                format!(":a :l (\"{}\").\n", y),
                "{ :a :l ?l. ?l list:iterate (?i ?m) } => { :a :out ?m }.",
                "{ :a :l ?l. ?l list:length ?n } => { :a :ok true }.",
                BIG,
                "the result of the builtin Iterate",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "list:first",
                format!(":a :l (\"{}\").\n", y),
                "{ :a :l ?l. ?l list:first ?m } => { :a :out ?m }.",
                "{ :a :l ?l. ?l list:length ?n } => { :a :ok true }.",
                BIG,
                "the result of the builtin First",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "list:last",
                format!(":a :l (\"{}\").\n", y),
                "{ :a :l ?l. ?l list:last ?m } => { :a :out ?m }.",
                "{ :a :l ?l. ?l list:length ?n } => { :a :ok true }.",
                BIG,
                "the result of the builtin Last",
            ),
        ),
        // A virtual rdf:rest of a list value; the control takes rdf:first by the same
        // path, so both hold the same copy of the list.
        checked(
            Check::Allocated,
            case(
                "rdf:rest of a list value",
                format!(":a :l (\"a\" \"{}\").\n", y),
                "{ :a :l ?l. ?l rdf:rest ?t } => { :a :out ?t }.",
                "{ :a :l ?l. ?l rdf:first ?h } => { :a :ok true }.",
                BIG,
                "the rest of a list (rdf:rest)",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "rdf:rest in a formula search",
                format!(":a :f {{ :s :p (\"a\" \"{}\") }}.\n", y),
                "{ :a :f ?f. ?f log:includes { :s :p ?l. ?l rdf:rest ?t } } => { :a :out true }.",
                "{ :a :f ?f. ?f log:includes { :s :p ?l. ?l rdf:first ?h } } => { :a :ok true }.",
                BIG,
                "the rest of a list (rdf:rest)",
            ),
        ),
        checked(
            Check::Allocated,
            case(
                "time:timeZone",
                format!(":a :p \"2020-01-01T00:00:00+{}\".\n", "0".repeat(BIG)),
                "{ :a :p ?s. ?s time:timeZone ?t } => { :a :out ?t }.",
                "{ :a :p ?s. ?s string:length ?n } => { :a :ok true }.",
                BIG,
                "the result of the builtin TimeZone",
            ),
        ),
        checked(
            Check::RefusalOnly,
            case(
                "log:content",
                String::new(),
                &format!("{{ <{BIG_DOC}> log:content ?c }} => {{ :a :out ?c }}."),
                NOTHING,
                BIG,
                "the result of the builtin Content",
            ),
        ),
        // The formula log:conclusion returns: two rule statements of 40 KiB each. Closing
        // it copies them several times over, so the bytes are counted from the closure's
        // last read of LATE_DOC (its third rule). The control closes the same formula for
        // log:supports and derives nothing.
        checked(
            Check::AfterLastRead,
            case(
                "log:conclusion",
                format!(
                    ":a :f {{ {{ :s :p \"{x}\" }} => {{ :s :q :r }}. \
                     {{ :s :p \"{x}y\" }} => {{ :s :q :r }}. \
                     {{ <{LATE_DOC}> log:content ?m }} => {{ :s :q :r }} }}.\n"
                ),
                "{ :a :f ?f. ?f log:conclusion ?c } => { :a :out ?c }.",
                "{ :a :f ?f. ?f log:supports { :no :such :fact } } => { :a :ok true }.",
                2 * S,
                "the result of the builtin LogConclusion",
            ),
        ),
    ]
}

#[test]
fn every_derived_term_is_refused_before_it_is_built() {
    // Every case runs; the failures are reported together.
    let mut report = Vec::new();
    let mut failed = Vec::new();
    for c in cases() {
        // Every document holds `:a :ok true`, which each control derives: a control runs
        // one round and stops, as a refused run does.
        let doc = format!("{PREFIXES}:a :ok true.\n{}{}\n", c.facts, marked(&c.rule));
        let control = format!(
            "{PREFIXES}:a :ok true.\n{}{}\n",
            c.facts,
            marked(&c.control)
        );
        let (ctl, ctl_m) = measured(&control);
        assert!(ctl.is_ok(), "{}: control failed: {ctl:?}", c.what);
        let (r, m) = measured(&doc);
        report.push(format!(
            "{}: peak {} B vs control {} B; allocated {} B vs control {} B",
            c.what, m.peak, ctl_m.peak, m.total, ctl_m.total
        ));
        match &r {
            Err(e) if e.contains("max_term_bytes") && e.contains(c.names) => {}
            Err(e) => failed.push(format!(
                "{}: refused for another reason (want {:?}): {e}",
                c.what, c.names
            )),
            Ok(n) => failed.push(format!("{}: not refused ({n} facts)", c.what)),
        }
        let half = c.refused / 2;
        let built_first = match c.check {
            Check::Peak => (m.peak >= ctl_m.peak + half)
                .then(|| format!("peak heap {} B, control {} B", m.peak, ctl_m.peak)),
            Check::Allocated => (m.total >= ctl_m.total + half)
                .then(|| format!("allocated {} B, control {} B", m.total, ctl_m.total)),
            Check::AfterLastRead => (m.total >= half)
                .then(|| format!("allocated {} B after the closure's last read", m.total)),
            Check::RefusalOnly => None,
        };
        if let Some(why) = built_first {
            failed.push(format!(
                "{}: {why}; the refused term ({} B) was built before it was sized",
                c.what, c.refused
            ));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
