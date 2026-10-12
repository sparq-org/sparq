//! GH #6771: every term the N3 text engine derives is sized BEFORE it is built. This
//! binary counts live heap bytes with a counting global allocator and drives each
//! builtin's result path (and the other derived-term paths: substitution, aggregation,
//! conclusions) past a small `max_term_bytes`: the call must fail with the closure-limit
//! error, and its peak heap must stay within half the refused term's size of a control
//! run that binds the same inputs but derives nothing large. A path that built the term
//! and checked it afterwards would show the whole term in its peak.
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

const PREFIXES: &str = "@prefix : <http://ex/>.\n\
     @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
     @prefix list: <http://www.w3.org/2000/10/swap/list#>.\n\
     @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n";

/// The closure of `doc` under [`LIMIT`], and the peak heap above the live heap at the
/// start of the call.
fn measured(doc: &str) -> (Result<usize, String>, usize) {
    let mut limits = ClosureLimits::default();
    limits.max_term_bytes = LIMIT;
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let r = reason_n3_terms_with_limits(doc, None, None, NegationCycles::Reject, limits)
        .map(|c| c.facts.len());
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(base);
    (r, peak)
}

struct Case {
    what: &'static str,
    facts: String,
    /// The rule that derives the oversized term.
    rule: String,
    /// The same premise (and, for a unary builtin, the same builtin input), deriving
    /// nothing large.
    control: String,
    /// The size of the term that is refused.
    refused: usize,
}

fn case(what: &'static str, facts: String, rule: &str, control: &str, refused: usize) -> Case {
    Case { what, facts, rule: rule.into(), control: control.into(), refused }
}

fn cases() -> Vec<Case> {
    let x = "x".repeat(S);
    let p = format!(":a :p \"{x}\".\n");
    let ok = "{ :a :p ?s } => { :a :ok true }.";
    // A unary builtin's evaluation holds copies of its (admitted) input; the control
    // passes the same input through a builtin with a small result.
    let unary = "{ :a :p ?s. ?s string:length ?n } => { :a :ok ?n }.";
    vec![
        case("string:concatenation", p.clone(), "{ :a :p ?s. (?s ?s) string:concatenation ?t } => { :a :out ?t }.", ok, 2 * S),
        case("string:format", p.clone(), "{ :a :p ?s. (\"%s%s\" ?s ?s) string:format ?t } => { :a :out ?t }.", ok, 2 * S),
        case("string:replace", p.clone(), "{ :a :p ?s. (?s \"x\" \"xx\") string:replace ?t } => { :a :out ?t }.", ok, 2 * S),
        case(
            "string:encodeForUri",
            format!(":a :p \"{}\".\n", " ".repeat(S)),
            "{ :a :p ?s. ?s string:encodeForUri ?t } => { :a :out ?t }.",
            unary,
            3 * S,
        ),
        case(
            "string:upperCase",
            // U+0390 (2 bytes) upper-cases to three characters (6 bytes).
            format!(":a :p \"{}\".\n", "\u{390}".repeat(S / 2)),
            "{ :a :p ?s. ?s string:upperCase ?t } => { :a :out ?t }.",
            unary,
            3 * S,
        ),
        case(
            "log:langlit",
            format!("{p}:a :l \"{}\".\n", "y".repeat(S)),
            "{ :a :p ?s. :a :l ?l. (?s ?l) log:langlit ?t } => { :a :out ?t }.",
            "{ :a :p ?s. :a :l ?l } => { :a :ok true }.",
            2 * S,
        ),
        case(
            "log:dtlit",
            format!("{p}:a :d <http://ex/{}>.\n", "y".repeat(S)),
            "{ :a :p ?s. :a :d ?d. (?s ?d) log:dtlit ?t } => { :a :out ?t }.",
            "{ :a :p ?s. :a :d ?d } => { :a :ok true }.",
            2 * S,
        ),
        case(
            "list:append",
            format!(":a :l (\"{x}\").\n"),
            "{ :a :l ?l. (?l ?l) list:append ?t } => { :a :out ?t }.",
            "{ :a :l ?l } => { :a :ok true }.",
            2 * S,
        ),
        case(
            "log:conjunction",
            format!(":a :f {{ :s :p \"{x}\" }}.\n"),
            "{ :a :f ?f. (?f ?f) log:conjunction ?t } => { :a :out ?t }.",
            "{ :a :f ?f } => { :a :ok true }.",
            2 * S,
        ),
        case(
            "log:collectAllIn",
            p.clone(),
            "{ ((?x ?x) { :a :p ?x } ?all) log:collectAllIn _:t } => { :a :out ?all }.",
            "{ :a :p ?x } => { :a :ok true }.",
            2 * S,
        ),
        case("list:in", p.clone(), "{ :a :p ?s. ?m list:in ((?s ?s)) } => { :a :out ?m }.", ok, 2 * S),
        case("conclusion", p.clone(), "{ :a :p ?s } => { :a :out (?s ?s) }.", ok, 2 * S),
    ]
}

#[test]
fn every_derived_term_is_refused_before_it_is_built() {
    let mut report = Vec::new();
    for c in cases() {
        let doc = format!("{PREFIXES}{}{}\n", c.facts, c.rule);
        let control = format!("{PREFIXES}{}{}\n", c.facts, c.control);
        let (ctl, ctl_peak) = measured(&control);
        assert!(ctl.is_ok(), "{}: control failed: {ctl:?}", c.what);
        let (r, peak) = measured(&doc);
        match &r {
            Err(e) => assert!(e.contains("max_term_bytes"), "{}: {e}", c.what),
            Ok(n) => panic!("{}: not refused ({n} facts)", c.what),
        }
        report.push(format!("{}: peak {peak} vs control {ctl_peak}", c.what));
        assert!(
            peak < ctl_peak + c.refused / 2,
            "{}: peak heap {peak} B is {} B over the control's {ctl_peak} B; the refused \
             term ({} B) was built before it was sized\n{}",
            c.what,
            peak.saturating_sub(ctl_peak),
            c.refused,
            report.join("\n")
        );
    }
    eprintln!("{}", report.join("\n"));
}
