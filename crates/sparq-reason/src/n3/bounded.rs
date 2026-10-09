//! Every budgeted or fallible step of N3 evaluation, and the ONE way to use its result.
//!
//! A search that stops early (a depth limit, a step budget, a length cap, a regex that
//! cannot be compiled, a nested closure left unclosed) under-approximates what it
//! reports, and so does a builtin whose exact value its number tower cannot represent
//! (an `i128` overflow). Any negation or aggregation that later reads the store would then read
//! missing facts as absent. So every such step returns a [`Bounded`] value, which is
//! `#[must_use]` and whose contents are private to this module: the only way to get the
//! value out is [`settle`], which records a cut on the run's sticky [`Sink`] on the way.
//! The text engine's sink is the run's [`Truncation`] flag, which the negation gate
//! reads; the incremental counting engine's sink is its "fall back to the checked
//! engine" flag.
//!
//! Every limit constant lives here and is private. `tests/n3_bounded_discipline.rs`
//! fails if a limit or a raw `Regex::new` appears in another N3 evaluator source.

use std::cell::Cell;
use std::rc::Rc;

/// Backward (`<=`) proof search depth, counted in rule applications. It bounds runaway
/// recursion (e.g. a rule whose premise re-poses its own goal); a search it stops is a cut.
const BW_DEPTH: usize = 64;
/// Steps one formula-containment search may take.
const CONTAINMENT_BUDGET: usize = 100_000;
/// Cells one walk of an `rdf:first`/`rdf:rest` data list may visit.
const LIST_WALK_CAP: usize = 100_000;

/// The result of a budgeted or fallible step: the value it computed, and whether the
/// step was cut short (then the value is an under-approximation).
#[must_use = "a Bounded result must be settled, which records a cut on the run"]
pub(crate) struct Bounded<T> {
    value: T,
    cut: Option<&'static str>,
}

impl<T> Bounded<T> {
    /// A step that ran to completion.
    pub(crate) fn complete(value: T) -> Self {
        Bounded { value, cut: None }
    }

    /// A step that was cut short for `why`; `value` is what it found before the cut.
    pub(crate) fn cut(value: T, why: &'static str) -> Self {
        Bounded {
            value,
            cut: Some(why),
        }
    }
}

/// Where [`settle`] records a cut.
pub(crate) trait Sink {
    fn record(&self, why: &'static str);
}

/// A run's cut record: the first reason any search of the run was cut short. Every part
/// of one logical evaluation (nested closures, an explicit-strata pipeline, a query and
/// the closure it projects over) records into the SAME handle, and it is never reset.
///
/// The field is private and there is no `Default`: the only way to make a fresh record
/// is [`Truncation::top_level`], which a top-level entry point calls once. Every nested
/// evaluation takes its parent's handle (a `clone` shares the same record), so an inner
/// cut always reaches the parent.
#[derive(Clone)]
pub(crate) struct Truncation(Rc<Cell<Option<&'static str>>>);

impl Truncation {
    /// The cut record of a new TOP-LEVEL run. A nested run must reuse its parent's.
    pub(crate) fn top_level() -> Truncation {
        Truncation(Rc::default())
    }

    /// The first cut the run recorded, if any.
    pub(crate) fn get(&self) -> Option<&'static str> {
        self.0.get()
    }
}

impl Sink for Truncation {
    fn record(&self, why: &'static str) {
        if self.0.get().is_none() {
            self.0.set(Some(why));
        }
    }
}

/// The incremental counting engine's "this needs the checked engine" flag.
impl Sink for Cell<bool> {
    fn record(&self, _why: &'static str) {
        self.set(true);
    }
}

/// The ONLY way to use a [`Bounded`] value: records a cut on `sink`, then returns the
/// value.
pub(crate) fn settle<T>(sink: &impl Sink, b: Bounded<T>) -> T {
    if let Some(why) = b.cut {
        sink.record(why);
    }
    b.value
}

/// The depth a top-level backward proof search starts with.
pub(crate) const fn backward_depth() -> usize {
    BW_DEPTH
}

/// One more backward rule application at `depth` remaining. `Some(depth - 1)` when the
/// budget allows it; `None` when it is spent, which is a cut if a backward rule could
/// still have matched the goal (`could_match`).
pub(crate) fn backward_step(depth: usize, could_match: bool) -> Bounded<Option<usize>> {
    match depth.checked_sub(1) {
        Some(d) => Bounded::complete(Some(d)),
        None if could_match => Bounded::cut(None, "backward proof search reached its depth limit"),
        None => Bounded::complete(None),
    }
}

/// A fresh step budget for one formula-containment search.
pub(crate) struct StepBudget(usize);

impl StepBudget {
    pub(crate) fn containment() -> Self {
        StepBudget(CONTAINMENT_BUDGET)
    }

    /// Take one step; `false` once the budget is spent.
    pub(crate) fn take(&mut self) -> bool {
        match self.0.checked_sub(1) {
            Some(n) => {
                self.0 = n;
                true
            }
            None => false,
        }
    }

    /// The search result, cut if the budget ran out.
    pub(crate) fn finish<T>(self, value: T) -> Bounded<T> {
        if self.0 == 0 {
            Bounded::cut(
                value,
                "formula containment search exhausted its step budget",
            )
        } else {
            Bounded::complete(value)
        }
    }
}

/// Walk a list cell by cell: `next(cell)` yields `(member, rest)` for a cons cell, or
/// `None` for a malformed cell (no list: a completed walk that found nothing);
/// `is_nil(cell)` ends the list. A walk past the cap is a cut.
pub(crate) fn walk_list<C, M>(
    head: C,
    is_nil: impl Fn(&C) -> bool,
    mut next: impl FnMut(&C) -> Option<(M, C)>,
) -> Bounded<Option<Vec<M>>> {
    let mut out = Vec::new();
    let mut cur = head;
    for _ in 0..=LIST_WALK_CAP {
        if is_nil(&cur) {
            return Bounded::complete(Some(out));
        }
        let Some((m, rest)) = next(&cur) else {
            return Bounded::complete(None);
        };
        out.push(m);
        cur = rest;
    }
    Bounded::cut(None, "a data list walk passed its length cap")
}

/// Compile a regex pattern supplied at run time. A pattern the regex engine refuses
/// (a syntax error, or a resource limit such as nesting depth or compiled size) is a
/// cut: the builtin cannot decide whether the string matches.
pub(crate) fn regex(pattern: &str) -> Bounded<Option<regex::Regex>> {
    match regex::Regex::new(pattern) {
        Ok(re) => Bounded::complete(Some(re)),
        Err(_) => Bounded::cut(
            None,
            "a regex could not be compiled (syntax or resource limit)",
        ),
    }
}

/// Compile a regex pattern that is a compile-time constant of a compiled rule set: a
/// pattern the regex engine refuses makes the rule set an error, never a builtin that
/// silently fails every row.
#[cfg(feature = "compiled-rules")]
pub(crate) fn regex_or_refuse(pattern: &str) -> Result<regex::Regex, String> {
    regex::Regex::new(pattern).map_err(|e| {
        format!(
            "compiled-rules: string:scrape regex {pattern:?} cannot be compiled ({e}); \
             the rule set is refused rather than evaluated as never matching"
        )
    })
}

/// A [`Sink`] local to one builtin evaluation: every fallible step inside the builtin
/// settles into it, and [`Pending::finish`] hands the builtin's result back as a
/// [`Bounded`] its caller must settle on the run.
#[derive(Default)]
pub(crate) struct Pending(Cell<Option<&'static str>>);

impl Sink for Pending {
    fn record(&self, why: &'static str) {
        if self.0.get().is_none() {
            self.0.set(Some(why));
        }
    }
}

impl Pending {
    /// The builtin's result, cut if any step inside it was.
    pub(crate) fn finish<T>(self, value: T) -> Bounded<T> {
        Bounded {
            value,
            cut: self.0.get(),
        }
    }
}

/// Bracket nesting the N3 parser accepts (its recursive descent recurses per level).
const PARSE_DEPTH: usize = 4096;

/// A limit was hit. Only this module can make one (its fields are private), and the
/// only way to use it during evaluation is [`parse_n3`], which records it as a cut.
#[derive(Debug)]
pub(crate) struct LimitError {
    why: &'static str,
    message: String,
}

impl LimitError {
    /// The error text a public entry point reports.
    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

/// The parser entering nesting level `depth`: an error past [`PARSE_DEPTH`].
pub(crate) fn enter_nesting(depth: usize) -> Result<(), LimitError> {
    if depth <= PARSE_DEPTH {
        Ok(())
    } else {
        Err(LimitError {
            why: "an N3 document passed the parser's nesting limit",
            message: format!("nesting deeper than {PARSE_DEPTH}"),
        })
    }
}

/// Parse an N3 document inside the crate: the ONE route every crate-internal parse takes
/// (entry points, the incremental graph's rules, proof re-derivation, the fallback reparse,
/// `log:semantics` / `log:parsedAsN3`). The result is the public parser's, unchanged: a
/// syntax error and the nesting limit are both `Err` with the same text as before. A
/// document that hits a parser limit ([`LimitError`]) is also a cut, recorded on `cuts`.
pub(crate) fn parse_n3(
    src: &str,
    base: &str,
    cuts: &impl Sink,
) -> Result<super::parser::Parsed, String> {
    parse_n3_with_extra(src, base, std::iter::empty(), cuts)
}

/// [`parse_n3`], with `extra` statements appended AS TERMS
/// ([`super::parser::parse_with_extra_checked`]).
pub(crate) fn parse_n3_with_extra(
    src: &str,
    base: &str,
    extra: impl IntoIterator<Item = [super::Term; 3]>,
    cuts: &impl Sink,
) -> Result<super::parser::Parsed, String> {
    use super::parser::{parse_with_extra_checked, ParseFailure};
    let parsed = match parse_with_extra_checked(src, base, extra) {
        Ok(p) => Bounded::complete(Ok(p)),
        Err(ParseFailure::Syntax(m)) => Bounded::complete(Err(m)),
        Err(ParseFailure::Resource(e)) => Bounded::cut(Err(e.message), e.why),
    };
    settle(cuts, parsed)
}

/// A document a builtin asked the resolver for (`log:semantics`, `log:content`). No
/// document (no resolver, or one that cannot supply it) is a cut: its content is
/// unknown, not empty.
pub(crate) fn resolved(text: Option<String>) -> Bounded<Option<String>> {
    match text {
        Some(t) => Bounded::complete(Some(t)),
        None => Bounded::cut(None, "a document a builtin needed could not be resolved"),
    }
}

/// An all-ASCII-digit field of a lexical form as an `i64`. A field that is not all
/// digits is an ill-typed lexical form (spec-defined no-match); digits past the `i64`
/// range are a cut, since the value exists but cannot be represented.
pub(crate) fn digits_i64(x: &str) -> Bounded<Option<i64>> {
    if x.is_empty() || !x.bytes().all(|b| b.is_ascii_digit()) {
        return Bounded::complete(None);
    }
    match x.parse::<i64>() {
        Ok(v) => Bounded::complete(Some(v)),
        Err(_) => Bounded::cut(None, "a date/time field passed the i64 range"),
    }
}

/// Years the epoch arithmetic of the `time:` builtins accepts (so day and second
/// counts cannot overflow `i64`). A year past it is a cut.
const EPOCH_YEAR_CAP: i64 = 999_999_999;

/// The (unsigned) year field of a date lexical form, for the epoch arithmetic of the
/// `time:` builtins. Text that is not all ASCII digits is malformed: no value, and no
/// cut (the builtin is false by definition). A digit-only year past the range the
/// arithmetic accepts, however many digits, gives no value and is a cut: checked on the
/// digits, before any integer conversion, so even a year past `i64` is a cut.
pub(crate) fn epoch_year(field: &str) -> Bounded<Option<i64>> {
    if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
        return Bounded::complete(None);
    }
    let digits = field.trim_start_matches('0');
    let cap = EPOCH_YEAR_CAP.to_string();
    let within = digits.len() < cap.len() || (digits.len() == cap.len() && digits <= cap.as_str());
    match digits.parse::<i64>() {
        Ok(year) if within => Bounded::complete(Some(year)),
        _ if digits.is_empty() => Bounded::complete(Some(0)),
        _ => Bounded::cut(None, "a date/time year passed the epoch arithmetic's range"),
    }
}

/// One checked step of exact arithmetic. `None` means the exact value exists but the
/// `i128` tower cannot hold it (an overflow, an unrepresentable result): a cut.
pub(crate) fn exact<T>(step: Option<T>) -> Bounded<Option<T>> {
    match step {
        Some(v) => Bounded::complete(Some(v)),
        None => Bounded::cut(None, "exact arithmetic passed the i128 range"),
    }
}

/// A numeral the exact tower read as its `f64` image. An integer or decimal numeral
/// (no exponent, not `INF`/`NaN`) only lands there when its value is past `i128`: a cut.
pub(crate) fn exact_numeral(lex: &str) -> Bounded<()> {
    let body = lex.strip_prefix(['+', '-']).unwrap_or(lex);
    let digits = body.bytes().filter(u8::is_ascii_digit).count();
    let dots = body.bytes().filter(|&b| b == b'.').count();
    if digits > 0 && dots <= 1 && digits + dots == body.len() {
        Bounded::cut((), "an exact numeral passed the i128 range")
    } else {
        Bounded::complete(())
    }
}

/// A number used as a whole `i64` where only an exact value will do (a `time:inSeconds`
/// epoch second count): `whole` is its exact integer value, or `None` when it has a
/// fraction. A fraction, or a whole value past `i64`, cannot be used without losing
/// precision: a cut.
pub(crate) fn whole_i64(whole: Option<i128>) -> Bounded<Option<i64>> {
    match whole.map(i64::try_from) {
        Some(Ok(v)) => Bounded::complete(Some(v)),
        Some(Err(_)) => Bounded::cut(None, "a number passed the i64 range"),
        None => Bounded::cut(
            None,
            "a number with a fraction was used where only a whole number is exact",
        ),
    }
}

/// An `f64` used as a whole `i64` exactly ([`whole_i64`]). `NaN` is no number (no
/// match); a fraction, an infinity or a value past `i64` is a cut.
pub(crate) fn whole_i64_of_f64(n: f64) -> Bounded<Option<i64>> {
    if n.is_nan() {
        Bounded::complete(None)
    } else if n.is_finite() && n.fract() != 0.0 {
        whole_i64(None)
    } else if n.is_finite() && n >= i64::MIN as f64 && n < i64::MAX as f64 {
        Bounded::complete(Some(n as i64))
    } else {
        Bounded::cut(None, "a number passed the i64 range")
    }
}

/// The fraction digits of a seconds field that a whole-second result drops
/// (`time:inSeconds` over `…:01.5Z`): a nonzero fraction is lost precision, a cut.
pub(crate) fn dropped_fraction(frac: &str) -> Bounded<()> {
    if frac.bytes().all(|b| b == b'0') {
        Bounded::complete(())
    } else {
        Bounded::cut(
            (),
            "a fractional second was dropped from a whole-second result",
        )
    }
}

/// The integer part of an `f64` as an `i64`. `NaN` has none (no match); an infinite or
/// out-of-range value has one that `i64` cannot hold: a cut.
pub(crate) fn int_of_f64(n: f64) -> Bounded<Option<i64>> {
    if n.is_nan() {
        Bounded::complete(None)
    } else if n.is_finite() && n >= i64::MIN as f64 && n < i64::MAX as f64 {
        Bounded::complete(Some(n as i64))
    } else {
        Bounded::cut(None, "a number passed the i64 range")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut_of<T>(b: Bounded<T>) -> (T, bool) {
        let sink = Cell::new(false);
        let v = settle(&sink, b);
        (v, sink.get())
    }

    #[test]
    fn each_limit_reports_a_cut_and_keeps_its_value() {
        assert_eq!(cut_of(backward_step(3, true)), (Some(2), false));
        assert_eq!(cut_of(backward_step(0, true)), (None, true));
        assert_eq!(cut_of(backward_step(0, false)), (None, false));

        let mut b = StepBudget::containment();
        while b.take() {}
        assert_eq!(cut_of(b.finish(7)), (7, true));
        let mut b = StepBudget::containment();
        assert!(b.take());
        assert_eq!(cut_of(b.finish(7)), (7, false));

        let short = walk_list(3usize, |c| *c == 0, |c| Some((*c, c - 1)));
        assert_eq!(cut_of(short), (Some(vec![3, 2, 1]), false));
        let endless = walk_list(0usize, |_| false, |c| Some((*c, c + 1)));
        assert_eq!(cut_of(endless), (None, true));
        let malformed = walk_list(1usize, |_| false, |_| None::<(usize, usize)>);
        assert_eq!(cut_of(malformed), (None, false));

        assert!(cut_of(regex("a+")).0.is_some());
        let (re, cut) = cut_of(regex("("));
        assert!(re.is_none() && cut);

        assert!(enter_nesting(PARSE_DEPTH).is_ok());
        let e = enter_nesting(PARSE_DEPTH + 1).expect_err("past the limit");
        assert_eq!(e.message(), "nesting deeper than 4096");
        let sink = Cell::new(false);
        assert!(parse_n3(":a :b :c .", "", &sink).is_ok());
        assert!(parse_n3(":a :b", "", &sink).is_err());
        assert!(!sink.get(), "a syntax error is an error, not a cut");
        let deep = format!(
            ":a :b {}1{} .",
            "(".repeat(PARSE_DEPTH + 1),
            ")".repeat(PARSE_DEPTH + 1)
        );
        let deep = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let sink = Cell::new(false);
                let doc = parse_n3(&deep, "", &sink);
                (doc.err(), sink.get())
            })
            .expect("spawn")
            .join()
            .expect("join");
        let deep = (
            deep.0.as_deref() == Some("nesting deeper than 4096"),
            deep.1,
        );
        assert_eq!(deep, (true, true), "the nesting limit is a cut");

        assert_eq!(cut_of(epoch_year("2024")), (Some(2024), false));
        assert_eq!(cut_of(epoch_year("0000")), (Some(0), false));
        assert_eq!(
            cut_of(epoch_year("0999999999")),
            (Some(EPOCH_YEAR_CAP), false)
        );
        assert_eq!(cut_of(epoch_year("1000000000")), (None, true));
        assert_eq!(
            cut_of(epoch_year("9223372036854775808")),
            (None, true),
            "past i64"
        );
        assert_eq!(cut_of(epoch_year("20x4")), (None, false), "malformed");
        assert_eq!(cut_of(epoch_year("")), (None, false), "malformed");
    }

    #[test]
    fn a_lossy_whole_number_conversion_is_a_cut() {
        // 2^53 + 1 is exact as an integer; its f64 image is not.
        let big: i128 = 9_007_199_254_740_993;
        assert_eq!(
            cut_of(whole_i64(Some(big))),
            (Some(9_007_199_254_740_993), false)
        );
        assert_eq!(
            cut_of(whole_i64(Some(i128::from(i64::MAX) + 1))),
            (None, true)
        );
        assert_eq!(cut_of(whole_i64(None)), (None, true));
        assert_eq!(cut_of(whole_i64_of_f64(1e3)), (Some(1000), false));
        assert_eq!(cut_of(whole_i64_of_f64(1.5)), (None, true));
        assert_eq!(cut_of(whole_i64_of_f64(f64::INFINITY)), (None, true));
        assert_eq!(cut_of(whole_i64_of_f64(f64::NAN)), (None, false));
        assert_eq!(cut_of(dropped_fraction("000")), ((), false));
        assert_eq!(cut_of(dropped_fraction("5")), ((), true));
    }
}
