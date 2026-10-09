//! Every budgeted or fallible step of N3 evaluation, and the ONE way to use its result.
//!
//! A search that stops early (a depth limit, a step budget, a length cap, a regex that
//! cannot be compiled, a nested closure left unclosed) under-approximates what it
//! reports. Any negation or aggregation that later reads the store would then read
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
