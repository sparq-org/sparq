//! GH #6771: every term the N3 text engine derives is sized BEFORE it is built. This
//! binary counts live heap bytes with a counting global allocator and drives each
//! builtin's result path (and the other derived-term paths: substitution, aggregation,
//! conclusions) past a small `max_term_bytes`: the call must fail with the closure-limit
//! error naming that path, and its peak heap must stay within half the refused term's
//! size of a control run that holds the same inputs but derives nothing large. A path
//! that built the term and checked it afterwards would show the whole term in its peak.
//! Each refused term is many times the inputs the evaluation holds copies of, so those
//! copies cannot hide it; the exceptions are marked [`refusal_only`].
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

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwards the caller's layout to the system allocator unchanged.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
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
     @prefix xsd: <http://www.w3.org/2001/XMLSchema#>.\n";

/// The live heap when rule evaluation started, set by the first resolver read.
static MARK: AtomicUsize = AtomicUsize::new(usize::MAX);

/// The document every rule's first premise reads, marking the start of evaluation.
const MARK_PREMISE: &str = "<http://ex/mark> log:content ?mark. ";

/// `rule` with [`MARK_PREMISE`] as its first premise.
fn marked(rule: &str) -> String {
    let body = rule.strip_prefix("{ ").expect("a rule starts with `{ `");
    format!("{{ {MARK_PREMISE}{body}")
}

/// The closure of `doc` under [`LIMIT`], and the peak heap above the live heap at the
/// first resolver read (the start of rule evaluation).
fn measured(doc: &str) -> (Result<usize, String>, usize) {
    let mut limits = ClosureLimits::default();
    limits.max_term_bytes = LIMIT;
    MARK.store(usize::MAX, Ordering::Relaxed);
    let mark = |_: &str| -> Option<String> {
        if MARK.load(Ordering::Relaxed) == usize::MAX {
            let live = LIVE.load(Ordering::Relaxed);
            MARK.store(live, Ordering::Relaxed);
            PEAK.store(live, Ordering::Relaxed);
        }
        Some(String::new())
    };
    let r = reason_n3_terms_with_limits(doc, None, Some(&mark), NegationCycles::Reject, limits)
        .map(|c| c.facts.len());
    let base = MARK.load(Ordering::Relaxed);
    assert_ne!(base, usize::MAX, "no rule read the mark document:\n{doc}");
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(base);
    (r, peak)
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
    /// Whether the peak heap is checked: see [`refusal_only`].
    peak: bool,
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
        peak: true,
    }
}

/// A case whose refused term is at most three times an input the evaluation already holds
/// copies of (case mapping, URI encoding). Its peak heap cannot tell a term built before
/// it was sized from those copies, so only the refusal is checked; such a term is at most
/// three times an admitted one in any case.
fn refusal_only(c: Case) -> Case {
    Case { peak: false, ..c }
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
            &format!("{{ ({doubles}) list:length ?n }} => {{ :a :ok ?n }}."),
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
                "{{ (\"{}\" {doubles}) list:length ?n }} => {{ :a :ok ?n }}.",
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
            "{ :a :p ?s. (?s \"x\" \"y\") list:length ?n } => { :a :ok ?n }.",
            K * SMALL,
            "the result of the builtin Replace",
        ),
        refusal_only(case(
            "string:encodeForUri",
            String::new(),
            &format!(
                "{{ \"{}\" string:encodeForUri ?t }} => {{ :a :out ?t }}.",
                " ".repeat(S)
            ),
            &format!(
                "{{ \"{}\" string:length ?n }} => {{ :a :ok ?n }}.",
                " ".repeat(S)
            ),
            3 * S,
            "the result of the builtin EncodeForUri",
        )),
        refusal_only(case(
            "string:encodeForURI",
            String::new(),
            &format!(
                "{{ \"{}\" string:encodeForURI ?t }} => {{ :a :out ?t }}.",
                " ".repeat(S)
            ),
            &format!(
                "{{ \"{}\" string:length ?n }} => {{ :a :ok ?n }}.",
                " ".repeat(S)
            ),
            3 * S,
            "the result of the builtin EncodeForUriCwm",
        )),
        refusal_only(case(
            "string:encodeForFragID",
            String::new(),
            &format!(
                "{{ \"{}\" string:encodeForFragID ?t }} => {{ :a :out ?t }}.",
                " ".repeat(S)
            ),
            &format!(
                "{{ \"{}\" string:length ?n }} => {{ :a :ok ?n }}.",
                " ".repeat(S)
            ),
            3 * S,
            "the result of the builtin EncodeForFragId",
        )),
        // U+0390 (2 bytes) upper-cases to three characters (6 bytes).
        case(
            "string:upperCase",
            String::new(),
            &format!(
                "{{ \"{}\" string:upperCase ?t }} => {{ :a :out ?t }}.",
                "\u{390}".repeat(S / 2)
            ),
            &format!(
                "{{ \"{}\" string:length ?n }} => {{ :a :ok ?n }}.",
                "\u{390}".repeat(S / 2)
            ),
            3 * S,
            "the result of the builtin UpperCase",
        ),
        // U+0130 (2 bytes) lower-cases to two characters (3 bytes).
        refusal_only(case(
            "string:lowerCase",
            String::new(),
            &format!(
                "{{ \"{}\" string:lowerCase ?t }} => {{ :a :out ?t }}.",
                "\u{130}".repeat(24 << 10)
            ),
            &format!(
                "{{ \"{}\" string:length ?n }} => {{ :a :ok ?n }}.",
                "\u{130}".repeat(24 << 10)
            ),
            72 << 10,
            "the result of the builtin LowerCase",
        )),
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
            "{ (:c :c) list:length ?n } => { :a :ok ?n }.",
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
    ]
}

#[test]
fn every_derived_term_is_refused_before_it_is_built() {
    // Every case runs; the failures are reported together.
    let mut report = Vec::new();
    let mut failed = Vec::new();
    for c in cases() {
        let doc = format!("{PREFIXES}{}{}\n", c.facts, marked(&c.rule));
        let control = format!("{PREFIXES}{}{}\n", c.facts, marked(&c.control));
        let (ctl, ctl_peak) = measured(&control);
        assert!(ctl.is_ok(), "{}: control failed: {ctl:?}", c.what);
        let (r, peak) = measured(&doc);
        report.push(format!("{}: peak {peak} B vs control {ctl_peak} B", c.what));
        match &r {
            Err(e) if e.contains("max_term_bytes") && e.contains(c.names) => {}
            Err(e) => failed.push(format!(
                "{}: refused for another reason (want {:?}): {e}",
                c.what, c.names
            )),
            Ok(n) => failed.push(format!("{}: not refused ({n} facts)", c.what)),
        }
        if c.peak && peak >= ctl_peak + c.refused / 2 {
            failed.push(format!(
                "{}: peak heap {peak} B is {} B over the control's {ctl_peak} B; the refused term \
                 ({} B) was built before it was sized",
                c.what,
                peak.saturating_sub(ctl_peak),
                c.refused
            ));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
