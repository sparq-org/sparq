//! Every limit of N3 evaluation, and the ONE way to use what a limited step returns.
//!
//! A search that stops at a limit (a depth limit, a step budget, a length cap, a regex
//! the engine refuses, a year past the epoch arithmetic's range) under-approximates
//! what it reports. Each such step returns a [`Bounded`] value, which is `#[must_use]`
//! and whose contents are private to this module: the only way to get the value out is
//! [`settle`], which records the cut on a [`Sink`] on the way.
//!
//! Every limit constant lives here and is private, so no evaluator can reach a limit
//! except through this module. `tests/n3_bounded_discipline.rs` fails if a limit name,
//! a large limit literal or a raw `Regex::new` appears in another N3 evaluator source.
//!
//! Recording a cut changes no result: each limited step returns exactly what it
//! returned before this module existed. What a run does with its recorded cuts is a
//! separate decision.

use std::cell::Cell;
use std::rc::Rc;

/// Backward (`<=`) proof search depth, counted in rule applications. It bounds runaway
/// recursion (e.g. a rule whose premise re-poses its own goal); within the bound,
/// proofs are exhaustive.
const BW_DEPTH: usize = 64;
/// Steps one formula-containment search may take.
const CONTAINMENT_BUDGET: usize = 100_000;
/// Cells one walk of an `rdf:first`/`rdf:rest` data list may visit.
const LIST_WALK_CAP: usize = 100_000;
/// Bracket nesting the N3 parser accepts (its recursive descent recurses per level).
const PARSE_DEPTH: usize = 4096;
/// Years the epoch arithmetic of the `time:` builtins accepts, so that day and second
/// counts cannot overflow `i64`.
const EPOCH_YEAR_CAP: i64 = 999_999_999;

/// The result of a limited step: the value it computed, and whether the step was cut
/// short at a limit (then the value is an under-approximation).
#[must_use = "a Bounded result must be settled, which records a cut"]
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

/// The first reason any step of a run was cut short. Shared by every part of one
/// logical evaluation (nested closures included) and never reset.
pub(crate) type Cuts = Rc<Cell<Option<&'static str>>>;

impl Sink for Cuts {
    fn record(&self, why: &'static str) {
        if self.get().is_none() {
            self.set(Some(why));
        }
    }
}

/// A plain flag, e.g. the incremental counting engine's "this needs the checked
/// engine" flag.
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

/// One more backward rule application at `depth` remaining: `Some(depth - 1)` when the
/// budget allows it, `None` when it is spent. A spent budget is a cut if a backward rule
/// could still have matched the goal (`could_match`).
pub(crate) fn backward_step(depth: usize, could_match: bool) -> Bounded<Option<usize>> {
    match depth.checked_sub(1) {
        Some(d) => Bounded::complete(Some(d)),
        None if could_match => Bounded::cut(None, "backward proof search reached its depth limit"),
        None => Bounded::complete(None),
    }
}

/// A step budget for one formula-containment search.
pub(crate) struct StepBudget(usize);

impl StepBudget {
    /// A fresh budget for one containment search.
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

/// Walk a data list cell by cell: `next(cell)` yields `(member, rest)` for a cons cell,
/// or `None` for a malformed cell (no list: a completed walk that found nothing);
/// `is_nil(cell)` ends the list. A walk past the cap finds no list, and is a cut.
pub(crate) fn walk_list<C, M>(
    head: C,
    is_nil: impl Fn(&C) -> bool,
    mut next: impl FnMut(&C) -> Option<(M, C)>,
) -> Bounded<Option<Vec<M>>> {
    let mut out = Vec::new();
    let mut cur = head;
    loop {
        if is_nil(&cur) {
            return Bounded::complete(Some(out));
        }
        if out.len() > LIST_WALK_CAP {
            return Bounded::cut(None, "a data list walk passed its length cap");
        }
        let Some((m, rest)) = next(&cur) else {
            return Bounded::complete(None);
        };
        out.push(m);
        cur = rest;
    }
}

/// Compile a regex pattern. A pattern the regex engine refuses (a syntax error, or one
/// of its resource limits such as nesting depth or compiled size) gives no regex, and
/// is a cut: the caller cannot decide whether a string matches.
pub(crate) fn regex(pattern: &str) -> Bounded<Option<regex::Regex>> {
    match regex::Regex::new(pattern) {
        Ok(re) => Bounded::complete(Some(re)),
        Err(_) => Bounded::cut(
            None,
            "a regex could not be compiled (syntax or resource limit)",
        ),
    }
}

/// Whether the parser may enter one more nesting level at `depth`.
pub(crate) fn nesting_allowed(depth: usize) -> bool {
    depth <= PARSE_DEPTH
}

/// The parser's error for a document nested past [`nesting_allowed`].
pub(crate) fn nesting_error() -> String {
    format!("nesting deeper than {PARSE_DEPTH}")
}

/// `year` if the epoch arithmetic of the `time:` builtins can represent it; a year past
/// it gives no value, and is a cut.
pub(crate) fn epoch_year(year: i64) -> Bounded<Option<i64>> {
    if year <= EPOCH_YEAR_CAP {
        Bounded::complete(Some(year))
    } else {
        Bounded::cut(None, "a date/time year passed the epoch arithmetic's range")
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

        assert!(nesting_allowed(PARSE_DEPTH) && !nesting_allowed(PARSE_DEPTH + 1));
        assert_eq!(nesting_error(), "nesting deeper than 4096");

        assert_eq!(cut_of(epoch_year(2024)), (Some(2024), false));
        assert_eq!(cut_of(epoch_year(EPOCH_YEAR_CAP + 1)), (None, true));
    }
}
