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

/// The first reason any search of a run was cut short. Shared by every part of one
/// logical evaluation (nested closures, an explicit-strata pipeline, a query and the
/// closure it projects over) and never reset.
pub(crate) type Truncation = Rc<Cell<Option<&'static str>>>;

impl Sink for Truncation {
    fn record(&self, why: &'static str) {
        if self.get().is_none() {
            self.set(Some(why));
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

/// Whether the parser may enter one more nesting level at `depth`.
pub(crate) fn nesting_allowed(depth: usize) -> bool {
    depth <= PARSE_DEPTH
}

/// Parse an N3 document a builtin reads at run time (`log:semantics`,
/// `log:parsedAsN3`). A syntax error is the builtin's spec-defined failure (the text is
/// not N3: no match); a document the parser stops on a resource limit (nesting depth)
/// is a cut.
pub(crate) fn parse_n3(src: &str, base: &str) -> Bounded<Option<super::parser::Parsed>> {
    match super::parser::parse_with_base_limited(src, base) {
        (Ok(p), _) => Bounded::complete(Some(p)),
        (Err(_), true) => Bounded::cut(
            None,
            "an N3 document a builtin parsed passed the parser's nesting limit",
        ),
        (Err(_), false) => Bounded::complete(None),
    }
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

/// `year` if the epoch arithmetic can represent it.
pub(crate) fn epoch_year(year: i64) -> Bounded<Option<i64>> {
    if year <= EPOCH_YEAR_CAP {
        Bounded::complete(Some(year))
    } else {
        Bounded::cut(None, "a date/time year passed the epoch arithmetic's range")
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
