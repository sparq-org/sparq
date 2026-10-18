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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::rc::Rc;

use super::model::Term;

/// Backward (`<=`) proof search depth, counted in rule applications. It bounds runaway
/// recursion (e.g. a rule whose premise re-poses its own goal); a search it stops is a cut.
const BW_DEPTH: usize = 64;
/// Steps one formula-containment search may take.
const CONTAINMENT_BUDGET: usize = 100_000;
/// Pairing attempts the unifications of one budget may make past the free allowances of
/// their formula matches ([`StepBudget::unification`]).
const UNIFICATION_BUDGET: usize = 100_000;
/// Why a unification was cut short.
const UNIFICATION_CUT: &str = "formula unification exhausted its step budget";
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

    /// The same step, its value mapped by `f` (the cut, if any, kept).
    pub(crate) fn map<U>(self, f: impl FnOnce(T) -> U) -> Bounded<U> {
        Bounded {
            value: f(self.value),
            cut: self.cut,
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
/// The record also carries the run's [`ClosureLimits`], so every forward closure of the
/// run (nested ones included) gets the allowance its entry point was given, the run's
/// stack allowance ([`RUN_STACK`]), so one allowance is shared by every level, and the
/// run's refusal: the error of the first term refused for its size ([`derived_term`]),
/// which fails every closure of the run that sees it.
///
/// The fields are private and there is no `Default`: the only ways to make a fresh record
/// are [`Truncation::top_level`] and [`Truncation::top_level_with`], which a top-level
/// entry point calls once. Every nested evaluation takes its parent's handle (a `clone`
/// shares the same record), so an inner cut always reaches the parent.
#[derive(Clone)]
pub(crate) struct Truncation {
    cut: Rc<Cell<Option<&'static str>>>,
    refusal: Rc<RefCell<Option<String>>>,
    limits: ClosureLimits,
    /// Units of the run's stack allowance held by steps in progress ([`RUN_STACK`]).
    stack: Rc<Cell<usize>>,
    /// How deep a term the run keeps ([`generated`]): [`TERM_DEPTH`], or the deepest term
    /// it was given ([`given_terms`]).
    term_depth: Rc<Cell<usize>>,
}

impl Truncation {
    /// The cut record of a new TOP-LEVEL run under the default [`ClosureLimits`]. A
    /// nested run must reuse its parent's.
    pub(crate) fn top_level() -> Truncation {
        Truncation::top_level_with(ClosureLimits::default())
    }

    /// The cut record of a new TOP-LEVEL run whose forward closures get `limits`.
    pub(crate) fn top_level_with(limits: ClosureLimits) -> Truncation {
        Truncation {
            cut: Rc::default(),
            refusal: Rc::default(),
            limits,
            stack: Rc::default(),
            term_depth: Rc::new(Cell::new(TERM_DEPTH)),
        }
    }

    /// The first cut the run recorded, if any.
    pub(crate) fn get(&self) -> Option<&'static str> {
        self.cut.get()
    }

    /// The error of the first term the run refused for its size, if any: every closure of
    /// the run that ends after it fails with it.
    pub(crate) fn refusal(&self) -> Option<String> {
        self.refusal.borrow().clone()
    }

    /// Refuse the run with `error` (the first refusal is kept) and record the cut.
    fn refuse(&self, error: impl FnOnce() -> String) {
        self.refusal.borrow_mut().get_or_insert_with(error);
        self.record(CLOSURE_CUT);
    }
}

impl Sink for Truncation {
    fn record(&self, why: &'static str) {
        if self.cut.get().is_none() {
            self.cut.set(Some(why));
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

/// One more backward rule application at `depth` remaining in the run `run`:
/// `Some(depth - 1)` and one unit of the run's stack allowance ([`RUN_STACK`], held until
/// the step's frames are dropped) when both allow it; `None` when either is spent, which
/// is a cut if a backward rule could still have matched the goal (`could_match`).
pub(crate) fn backward_step(
    run: &Truncation,
    depth: usize,
    could_match: bool,
) -> Bounded<Option<(usize, StackFrames)>> {
    let step = depth.checked_sub(1).map(|d| (d, take_stack(run, 1)));
    match step {
        Some((d, Some(frames))) => Bounded::complete(Some((d, frames))),
        None if could_match => Bounded::cut(None, "backward proof search reached its depth limit"),
        Some(_) if could_match => Bounded::cut(
            None,
            "backward proof search reached the run's stack allowance",
        ),
        _ => Bounded::complete(None),
    }
}

/// A step budget: for one formula-containment search, or for unifications.
///
/// A unification budget also carries a free allowance for the formula match it is in
/// ([`StepBudget::enter_formula`]): the pairings of a search that never backtracks.
/// Steps come from the free allowance first; it is dropped when the outermost formula
/// match ends, so it never carries over to another match or tops up `left`.
pub(crate) struct StepBudget {
    /// Steps left beyond the free allowance. Never added to.
    left: usize,
    /// The free allowance of the outermost formula match in progress.
    free: usize,
    /// Formula matches in progress, nested.
    depth: usize,
    /// A step was refused: the budget stays spent.
    spent: bool,
}

impl StepBudget {
    fn new(left: usize) -> Self {
        StepBudget {
            left,
            free: 0,
            depth: 0,
            spent: false,
        }
    }

    pub(crate) fn containment() -> Self {
        Self::new(CONTAINMENT_BUDGET)
    }

    /// The budget of unifications: `UNIFICATION_BUDGET` steps, plus each outermost
    /// formula match's free allowance.
    pub(crate) fn unification() -> Self {
        Self::new(UNIFICATION_BUDGET)
    }

    /// A formula match starts. The outermost one gets `free` steps for itself and every
    /// match nested in it.
    pub(crate) fn enter_formula(&mut self, free: usize) {
        if self.depth == 0 {
            self.free = free;
        }
        self.depth += 1;
    }

    /// A formula match ended. When it was the outermost, its unused free steps are
    /// dropped.
    pub(crate) fn exit_formula(&mut self) {
        self.depth -= 1;
        if self.depth == 0 {
            self.free = 0;
        }
    }

    /// Take one step; `false` once the budget is spent, and from then on.
    pub(crate) fn take(&mut self) -> bool {
        if self.spent {
            return false;
        }
        if let Some(n) = self.free.checked_sub(1) {
            self.free = n;
            return true;
        }
        match self.left.checked_sub(1) {
            Some(n) => {
                self.left = n;
                true
            }
            None => {
                self.spent = true;
                false
            }
        }
    }

    /// Mark the budget spent: every later step is refused.
    pub(crate) fn spend(&mut self) {
        self.spent = true;
    }

    /// The search result, cut if the budget ran out.
    pub(crate) fn finish<T>(self, value: T) -> Bounded<T> {
        if self.spent || self.left == 0 {
            Bounded::cut(
                value,
                "formula containment search exhausted its step budget",
            )
        } else {
            Bounded::complete(value)
        }
    }
}

/// A unification of the run `run` ran out of steps: it did not unify, which is a cut.
pub(crate) fn unification_exhausted(run: &Truncation) {
    run.record(UNIFICATION_CUT);
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

/// Default [`ClosureLimits::max_rounds`]. A semi-naive closure runs about one round per
/// derivation step on its longest chain, so a linear walk of N nodes needs about N rounds;
/// a rule set that derives a new, syntactically distinct fact every round without end
/// (GH #6757) is stopped here instead of running until memory runs out.
const CLOSURE_ROUNDS: usize = 1_000_000;
/// Default [`ClosureLimits::max_facts`]: facts one forward closure may hold, asserted and
/// derived together.
const CLOSURE_FACTS: usize = 50_000_000;
/// Default [`ClosureLimits::max_term_bytes`]: 16 MiB, far past any literal or list a
/// rule set builds on purpose, and small enough that a term which doubles each round
/// (GH #6771) is refused while the closure's copies of it still fit in memory.
const CLOSURE_TERM_BYTES: usize = 16 << 20;
/// Default [`ClosureLimits::max_join_rows`] (GH #6938): unbounded, so a closure accepted
/// before the allowance existed is still accepted. A caller opts in by setting a limit.
const CLOSURE_JOIN_ROWS: usize = usize::MAX;
/// Default [`ClosureLimits::max_total_join_rows`]: unbounded, for the same reason the
/// per-round default is. A caller opts in by setting a limit.
const CLOSURE_TOTAL_JOIN_ROWS: usize = usize::MAX;

/// Why a forward closure was stopped by its [`ClosureLimits`].
const CLOSURE_CUT: &str = "forward closure limit exceeded";

/// The allowance of one N3 forward closure: how many rounds it may run, how many facts it
/// may hold, how large a term a functional builtin may derive, and how many binding rows
/// and rule firings one round may produce, before it is refused.
///
/// A rule set that never reaches a fixpoint (each round derives a new, syntactically
/// distinct fact) would otherwise run forever, and one whose derived terms grow (a string
/// that doubles each round, GH #6771) would exhaust memory within a few dozen rounds and
/// facts. When any allowance is spent the closure fails with an `Err` naming the limit
/// (`"forward closure limit exceeded"`) and records it as a cut, so no entry point returns
/// the truncated closure as a result. Each forward closure gets the allowance afresh: a
/// nested `log:conclusion` closure of a run gets the same limits as the run, not a share
/// of them.
///
/// `max_term_bytes` (GH #6771) bounds each term the text engine would build. These are
/// sized from borrowed inputs before they are built: every non-numeric builtin result (a
/// functional builtin's, `log:uri` either way, `log:dtlit` reversed, the member or pair a
/// list generator binds, the tail a virtual `rdf:rest` binds, the literal `log:content`
/// returns), a functional builtin's argument list, any term substituted under a rule's
/// bindings (a conclusion, a premise pattern, a scope or collection template, a
/// backward-rule projection), the list `log:collectAllIn` collects, and the formula
/// `log:conclusion` returns. The formula `log:parsedAsN3` / `log:semantics` parses is
/// charged by its parser before each allocation it keeps (every string, term node and
/// copy, directives included), and sized again once the parse returns it. A numeric
/// result (forward or reverse, a count or a date part) is sized from its computed value
/// (digit count, scale, sign, exponent) before its literal is rendered (GH #6901), and
/// `string:concatenation` sizes its numeric pieces the same way. The size is
/// approximate: string bytes plus a fixed cost per node. Binding a variable to a term
/// that already exists (premise matching, `log:includes`) copies that term; it builds
/// nothing new and is not sized again.
///
/// Substitution is sized because the raw substitution functions are private to this
/// module; the guard test `substitution_goes_through_the_bounded_module` is a lexical
/// tripwire for a crate-internal copy, not the guarantee.
///
/// `max_total_join_rows` bounds the SAME rows and firings over all rounds of one closure,
/// charged once and never refilled. Because the per-round allowance is refilled each round,
/// `max_join_rows` alone bounds a closure's work only as `max_join_rows × max_rounds`; a
/// caller that needs the whole closure's work bounded sets this one as well. The two
/// compose — whichever is reached first refuses the run, each naming itself — and it is
/// unbounded by default, so it changes nothing for a caller that does not set it.
///
/// `max_join_rows` (GH #6938) bounds the transient work of one round: the binding rows its
/// premise joins produce and the times its rules fire. Without it their number grows with
/// the product of the pattern matches (a premise of k unconstrained patterns over n facts
/// makes n^k rows) even when the distinct facts fit. The default is unbounded
/// (`usize::MAX`), so no closure accepted without the allowance is refused by default; set
/// a limit to opt in. Each round of a closure gets the allowance afresh, and every engine
/// charges the same two things:
/// * a **row** for each binding row a premise step that can multiply rows produces: a
///   join atom (each stored fact it matches, each backward proof of it, and the delta
///   facts a semi-naive round seeds it with), a list generator (`list:member`, `list:in`,
///   `list:iterate`), and each match a formula-scope search finds (`log:includes`,
///   `log:notIncludes`, `log:collectAllIn` and `log:forAllIn` over a written formula). A
///   step that keeps at most one row per input row (a filter, a functional builtin, the
///   outcome of a negation or an aggregation) cannot multiply them and is not charged. The
///   rows a store-scoped clause (`log:includes`, `log:notIncludes`, `log:collectAllIn`,
///   `log:forAllIn` over the store) or a backward rule's premise produces are charged the
///   same way, to the round that evaluates them;
/// * a **firing** for each conclusion triple a complete premise binding instantiates, and
///   for each conclusion the transitivity fast path instantiates, whether or not the fact
///   is new.
///
/// The text engine and the counting engine charge each row as it is produced, before it
/// is stored (a backward proof as it is projected onto its goal, a formula-scope match as
/// the search finds it), so a round never holds more charged rows than the allowance. The compiled
/// evaluator's hash join checks the allowance once per candidate fact it probes, so a
/// join stops at most one candidate's matches past it. Every engine charges a rule's
/// firings in one batch, once its premise is evaluated and before any conclusion is
/// instantiated. A round that would pass the allowance is refused like any other limit: a
/// cut, and the run's error naming `max_join_rows`. An N3 query
/// ([`reason_n3_query_terms`](crate::n3::reason_n3_query_terms)) charges the rows of its
/// premises over the closure as one more round (its answers are deduplicated as they are
/// built, so its firings are not charged).
///
/// What it does not bound:
/// * the closure's total memory: many distinct terms just under the limit are bounded
///   only by `max_facts`, and the rows of one round only by `max_join_rows`, which is
///   unbounded unless set;
/// * the steps of a formula-scope search beyond the matches it finds: each search is
///   bounded by its own step budget (`CONTAINMENT_BUDGET`, through
///   `StepBudget::containment`), and a search it stops is a cut;
/// * the pairings formula unification tries (a `{ … }` pattern matched against a
///   `{ … }` value, `formula_unify` in the text engine): it returns at most one binding,
///   so it adds no rows. Each unification has its own step budget
///   (`StepBudget::unification`): `UNIFICATION_BUDGET` steps, plus a free allowance for
///   its outermost formula match that covers a search that never backtracks and is
///   dropped when that match ends. Past it the terms do not unify and the run records a
///   cut. The unifications of one formula-containment search share one such budget, and
///   one that runs out stops that search as a cut (GH #6953);
/// * the document's own parse, which is not charged: its memory is bounded by its input
///   text. A `log:parsedAsN3` / `log:semantics` parse is charged; beyond its allowance it
///   holds only a scratch copy of one token, or an error message quoting one, at a time
///   (each bounded by the length of the source token), the spare capacity of its strings,
///   vectors and maps, its fixed-size state, and the text it parses;
/// * the text a resolver supplies (`log:content`, `log:semantics`), which the caller
///   allocates before it is sized (the literal or formula built from it is refused).
///
/// The incremental counting engine behind `MaterializedN3Graph` (GH #6854) takes the
/// same allowance (`MaterializedN3Graph::with_limits`; `new` uses the default). Its
/// builtins substitute their subject and object through `bounded::substitute` and size each
/// result with the text engine's own checkpoint before building it; its conclusions are
/// variables, IRIs and literals, so they build no new term. Each propagation is charged
/// one round per wave (and per round of a recursive layer's local fixpoint) against the
/// round allowance; each new distinct fact is checked against the fact allowance as it is
/// collected, and the closure after each wave, so no published closure holds more than
/// `max_facts` facts. Past any limit the graph rebuilds through the batch engine
/// under the same limits, so it holds the batch engine's closure or fails closed with its
/// error. Its rounds are not the batch engine's schedule, so under equal round or fact
/// limits it can hold a closure a from-scratch batch run near the boundary refuses. It
/// charges `max_join_rows` per wave (and per local round of a layer): each row a plain
/// atom, its seed or a negation guard produces, and each firing. A wave over a small delta
/// charges far less than a batch round over every fact, so the graph never refuses a
/// closure the batch engine accepts (a refusal rebuilds through it), but it keeps a
/// mutation whose wave fits even when a from-scratch batch round would not.
///
/// The opt-in `compiled-rules` evaluator (`n3::compiled`, GH #6855) takes the same
/// allowance (`eval_with_limits`; `eval` uses the default) and refuses with the same
/// error: each builtin result, and each argument list a builtin is evaluated over, is
/// sized the way the text engine sizes it before it is built, and its conclusions build
/// no new term (they copy ids the premise bound). It charges `max_join_rows` for each row
/// its hash join emits (before the join's repeated-variable check) and each firing, and
/// its first round of a stratum is naive, as the text engine's is. Its fixpoint schedule
/// differs (no closing naive check round, no transitivity fast path, no quoted-triple
/// substitution checkpoints, and a semi-naive round seeds a pattern from the delta in
/// place rather than first), so under equal limits the two engines can accept or refuse
/// differently near a boundary.
///
/// One over-refusal is deliberate: a builtin's argument list is a term it builds, so
/// `log:conjunction` over many copies of one large formula is refused when the copies
/// together pass the limit, even if the merged result would fit.
///
/// The struct is `#[non_exhaustive]`: outside this crate, start from
/// [`ClosureLimits::default`] and set the fields to change
/// (`let mut l = ClosureLimits::default(); l.max_term_bytes = 1 << 20;`).
///
/// Pass one to [`reason_n3_with_limits`](crate::n3::reason_n3_with_limits),
/// [`reason_n3_terms_with_limits`](crate::n3::reason_n3_terms_with_limits) or
/// [`MaterializedN3Graph::with_limits`](crate::MaterializedN3Graph::with_limits); every
/// other N3 entry point uses [`ClosureLimits::default`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClosureLimits {
    /// Rounds one forward closure may run (summed over its strata). Default 1,000,000.
    pub max_rounds: usize,
    /// Facts one forward closure may hold, asserted and derived together. Default
    /// 50,000,000.
    pub max_facts: usize,
    /// Approximate bytes of one term the closure may derive (a builtin's argument list or
    /// result, a parsed or concluded formula, a conclusion's compound term): its strings
    /// plus a fixed cost per node, counted through nested lists, formulas and quoted
    /// triples. The terms it covers are checked before they are built (see the type's
    /// docs). Default 16 MiB.
    pub max_term_bytes: usize,
    /// Binding rows and rule firings one round of the closure may produce, counted
    /// together: each row a join atom, list generator or `log:includes` produces, and each
    /// conclusion triple a rule instantiates (see the type's docs for exactly what is
    /// charged and when). Default unbounded (`usize::MAX`): set a limit to opt in.
    pub max_join_rows: usize,
    /// The same rows and firings `max_join_rows` counts, but summed over ALL rounds of one
    /// forward closure instead of one round: the total transient work the closure may do.
    ///
    /// `max_join_rows` alone bounds a closure's work only as the product of itself and
    /// `max_rounds`, because each round gets its per-round allowance afresh; a closure that
    /// stays just under the per-round limit for many rounds can therefore do far more work
    /// than either number suggests. This one is charged once and never refilled, so it is
    /// the whole closure's budget. The two compose: whichever is reached first refuses the
    /// run, each naming itself. Default unbounded (`usize::MAX`): set a limit to opt in, and
    /// a closure accepted before this field existed is still accepted.
    ///
    /// A nested closure gets its own total afresh, as it does its rounds and its per-round
    /// allowance.
    pub max_total_join_rows: usize,
}

impl Default for ClosureLimits {
    /// 1,000,000 rounds, 50,000,000 facts, 16 MiB per derived term, and unbounded join rows
    /// both per round and in total.
    fn default() -> Self {
        ClosureLimits {
            max_rounds: CLOSURE_ROUNDS,
            max_facts: CLOSURE_FACTS,
            max_term_bytes: CLOSURE_TERM_BYTES,
            max_join_rows: CLOSURE_JOIN_ROWS,
            max_total_join_rows: CLOSURE_TOTAL_JOIN_ROWS,
        }
    }
}

/// The join-work allowance of one round of a forward closure
/// ([`ClosureLimits::max_join_rows`], GH #6938): the binding rows and rule firings the
/// round may still produce. Each closure makes its own (a nested closure gets the
/// allowance afresh, as with its rounds) and calls [`JoinWork::round`] at the start of
/// each round; every row and firing is [charged](JoinWork::charge) before it is stored.
///
/// Spent, it refuses the run (the cut and the run's error, as [`derived_term`] does) and
/// stays spent: later rounds of the same closure produce nothing, so the closure reaches
/// its end quickly and fails with the refusal.
///
/// `usize::MAX`, the default, is the unbounded sentinel: no accounting at all, and every
/// charge succeeds, so the default can never refuse. A set limit is checked exactly.
pub(crate) struct JoinWork {
    left: Cell<usize>,
    limit: usize,
    /// What is left of the whole closure's total ([`ClosureLimits::max_total_join_rows`]) —
    /// charged alongside `left` and NEVER refilled by [`JoinWork::round`].
    total_left: Cell<usize>,
    total_limit: usize,
    spent: Cell<bool>,
    /// Both limits are the unbounded sentinel: charges are not counted.
    unbounded: bool,
}

impl JoinWork {
    /// The allowance of one round of a closure of the run `run`.
    pub(crate) fn forward(run: &Truncation) -> JoinWork {
        let limit = run.limits.max_join_rows;
        let total_limit = run.limits.max_total_join_rows;
        JoinWork {
            left: Cell::new(limit),
            limit,
            total_left: Cell::new(total_limit),
            total_limit,
            spent: Cell::new(false),
            unbounded: limit == usize::MAX && total_limit == usize::MAX,
        }
    }

    /// A new round: the full PER-ROUND allowance again, unless it was spent. The closure's
    /// total is not refilled — that is what makes it a total.
    pub(crate) fn round(&self) {
        if !self.spent.get() {
            self.left.set(self.limit);
        }
    }

    /// The join kernel's view of the rows the round may still produce: whichever of the
    /// round's allowance and the closure's remaining total is smaller.
    #[cfg(feature = "compiled-rules")]
    pub(crate) fn kernel_view(&self) -> RowsLeft {
        // Unbounded: `rows > usize::MAX` never holds, so the kernel never stops.
        RowsLeft(if self.unbounded {
            usize::MAX
        } else {
            self.left.get().min(self.total_left.get())
        })
    }

    /// Charge `n` rows or firings about to be stored: `true` when the round's allowance
    /// covers them. Otherwise the run is refused (recorded on `run`) and nothing may be
    /// stored. Under the unbounded default it always succeeds and counts nothing.
    #[inline]
    pub(crate) fn charge(&self, n: usize, run: &Truncation) -> bool {
        if self.unbounded {
            return true;
        }
        // Both allowances are charged, and neither is debited unless both cover `n`.
        match (self.left.get().checked_sub(n), self.total_left.get().checked_sub(n)) {
            (Some(left), Some(total_left)) => {
                self.left.set(left);
                self.total_left.set(total_left);
                true
            }
            (round_left, _) => {
                self.exceeded(run, round_left.is_some());
                false
            }
        }
    }

    /// Refuse the run: `total` says the CLOSURE's total was what it reached (the round's
    /// allowance still covered the charge), so the error names the limit that was actually
    /// spent.
    #[cold]
    fn exceeded(&self, run: &Truncation, total: bool) {
        self.left.set(0);
        self.total_left.set(0);
        self.spent.set(true);
        let limit = self.limit;
        let total_limit = self.total_limit;
        run.refuse(move || {
            if total {
                format!(
                    "n3: {CLOSURE_CUT}: the closure's premise joins and rule firings would \
                     produce more than max_total_join_rows ({total_limit}) binding rows over \
                     all its rounds: the rules keep joining patterns whose matches multiply \
                     (raise ClosureLimits::max_total_join_rows to allow more)"
                )
            } else {
                format!(
                    "n3: {CLOSURE_CUT}: one round's premise joins and rule firings would \
                     produce more than max_join_rows ({limit}) binding rows: the rules join \
                     patterns whose matches multiply (raise ClosureLimits::max_join_rows to \
                     allow more)"
                )
            }
        });
    }
}

/// The fixed cost [`term_bytes`] charges each node of a term.
pub(crate) const TERM_NODE_BYTES: usize = std::mem::size_of::<Term>();

/// The approximate size of `t` that [`ClosureLimits::max_term_bytes`] bounds: the bytes
/// of its strings plus [`TERM_NODE_BYTES`] per node, through nested lists, formulas and
/// quoted triples (walked with an explicit stack, so a deep term cannot overflow it).
pub(crate) fn term_bytes(t: &Term) -> usize {
    term_bytes_with(t, || {})
}

/// [`term_bytes`], calling `on_blank` once for each blank-node occurrence it walks.
pub(crate) fn term_bytes_with(t: &Term, mut on_blank: impl FnMut()) -> usize {
    let flat = |t: &Term| match t {
        Term::Iri(s) | Term::Blank(s) | Term::Var(s) => Some(s.len()),
        Term::Lit(v, dt, lang) => Some(v.len() + dt.len() + lang.as_ref().map_or(0, String::len)),
        Term::Formula(_) | Term::List(_) | Term::Triple(_) => None,
    };
    if let Some(n) = flat(t) {
        if matches!(t, Term::Blank(_)) {
            on_blank();
        }
        return n.saturating_add(TERM_NODE_BYTES);
    }
    let mut total = 0usize;
    let mut stack = vec![t];
    while let Some(t) = stack.pop() {
        total = total.saturating_add(TERM_NODE_BYTES);
        match t {
            Term::Formula(ts) => stack.extend(ts.iter().flatten()),
            Term::List(ms) => stack.extend(ms),
            Term::Triple(b) => stack.extend(b.iter()),
            leaf => {
                if matches!(leaf, Term::Blank(_)) {
                    on_blank();
                }
                total = total.saturating_add(flat(leaf).unwrap_or(0));
            }
        }
    }
    total
}

/// The compiled evaluator's join kernel's view of a round's [`JoinWork`]: the rows it may
/// still emit. Past them the kernel stops at its next candidate, and the evaluator's
/// charge of the rows it emitted refuses the run.
#[cfg(feature = "compiled-rules")]
pub(crate) struct RowsLeft(usize);

#[cfg(feature = "compiled-rules")]
impl sparq_substrate::join::Budget for RowsLeft {
    #[inline]
    fn exhausted(&self, rows: usize) -> bool {
        rows > self.0
    }
}

/// The [`term_bytes`] of an `xsd:string` literal whose lexical form has `lex` bytes.
pub(crate) fn string_term_bytes(lex: usize) -> usize {
    const XSD_STRING_LEN: usize = "http://www.w3.org/2001/XMLSchema#string".len();
    lex.saturating_add(XSD_STRING_LEN + TERM_NODE_BYTES)
}

/// The ONE checkpoint the terms the text engine sizes pass before they are built
/// (GH #6771, GH #6901). `bound` is an upper bound on the term's [`term_bytes`], computed
/// from borrowed inputs (or a computed numeric value) without building anything; `exact` is its exact size, computed (again
/// without building it) only when `bound` is over the limit, so a loose bound never
/// refuses a term that fits.
///
/// Past the run's [`ClosureLimits::max_term_bytes`] the term is refused (`true`): a cut,
/// and the run's refusal ([`Truncation::refusal`]), which fails every closure of the run,
/// so no entry point returns the closure without the term as a result.
pub(crate) fn derived_term(
    run: &Truncation,
    what: &dyn std::fmt::Display,
    bound: usize,
    exact: impl FnOnce() -> usize,
) -> Bounded<bool> {
    let limit = run.limits.max_term_bytes;
    if bound <= limit {
        return Bounded::complete(false);
    }
    let size = exact();
    if size <= limit {
        return Bounded::complete(false);
    }
    run.refuse(|| {
        format!(
            "n3: {CLOSURE_CUT}: {what} would be a term of about {size} bytes, over \
             max_term_bytes ({limit}): the rules build ever larger terms (raise \
             ClosureLimits::max_term_bytes to allow more)"
        )
    });
    Bounded::cut(true, CLOSURE_CUT)
}

/// What a substitution has to offer the term-building helpers below: the term a variable is
/// bound to, if it is bound.
///
/// Two substitutions implement it, and the helpers size and build the same terms for either.
/// The text engine's [`super::Binding`] keys itself by an `Rc<str>`, so copying a
/// substitution — which its backward proof search does per candidate site and per solution —
/// does not allocate a name per entry (GH #7080). The incremental counting engine carries a
/// plain `FxHashMap<String, Term>` and is left as it is. Both answer this one question, and
/// that is all these helpers ask of a substitution.
pub(crate) trait Subst {
    /// What `v` is bound to, or `None`.
    fn bound(&self, v: &str) -> Option<&Term>;
}

impl<S: BuildHasher> Subst for HashMap<String, Term, S> {
    fn bound(&self, v: &str) -> Option<&Term> {
        self.get(v)
    }
}

/// The [`term_bytes`] of `t` with `b`'s bindings substituted, as [`substitute`] (or, with
/// `deep`, [`substitute_deep`], through quoted formulae) would build it, read from the
/// borrowed terms without building anything.
pub(crate) fn resolved_bytes<B: Subst + ?Sized>(t: &Term, b: &B, deep: bool) -> usize {
    let all = |ts: &mut dyn Iterator<Item = &Term>| {
        ts.fold(TERM_NODE_BYTES, |n, m| n.saturating_add(resolved_bytes(m, b, deep)))
    };
    match t {
        Term::Var(v) => term_bytes(b.bound(v).unwrap_or(t)),
        Term::List(ms) => all(&mut ms.iter()),
        Term::Triple(tr) => all(&mut tr.iter()),
        Term::Formula(ts) if deep => all(&mut ts.iter().flatten()),
        _ => term_bytes(t),
    }
}

/// What a substitution refusal names.
/// What a virtual `rdf:rest` builds, for the refusal message.
struct ListTail;

impl std::fmt::Display for ListTail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the rest of a list (rdf:rest)")
    }
}

/// The list after `ms`'s first member, as a virtual `rdf:rest` of a list value binds it:
/// sized from the borrowed members before it is built, and `None` (with the refusal
/// recorded on `run`) past the limit. `ms` must not be empty.
pub(crate) fn list_tail(ms: &[Term], run: &Truncation) -> Option<Term> {
    let rest = &ms[1..];
    let bytes = rest.iter().map(term_bytes).fold(TERM_NODE_BYTES, usize::saturating_add);
    if settle(run, derived_term(run, &ListTail, bytes, || bytes)) {
        return None;
    }
    Some(Term::List(rest.to_vec()))
}

struct Substituted;

impl std::fmt::Display for Substituted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a term built under a rule's bindings")
    }
}

/// THE substitution of the text engine: `t` with `b`'s bound variables substituted
/// (recursing into lists and quoted triples; a quoted formula is a value, kept as is).
/// The incremental counting engine substitutes a builtin's subject and object through it
/// too (GH #6854), so both engines size the same terms.
/// A compound `t` can name a bound term many times, so it is sized from the borrowed
/// terms first ([`resolved_bytes`]) and refused past the run's term-size limit
/// ([`derived_term`]): `None`, with the cut and the run's refusal recorded. A variable or
/// atom is one copy of an existing term and is not sized. `tests/n3_bounded_discipline.rs`
/// fails if an N3 evaluator substitutes any other way.
#[inline]
pub(crate) fn substitute<B: Subst + ?Sized>(
    t: &Term,
    b: &B,
    run: &Truncation,
) -> Option<Term> {
    match t {
        Term::List(_) | Term::Triple(_) if compound_refused(t, b, run, false) => None,
        _ => Some(apply(t, b)),
    }
}

/// [`substitute`], also substituting inside quoted formulae (a conclusion's
/// instantiation): a variable left inside a formula after substitution is the formula's
/// own.
#[inline]
pub(crate) fn substitute_deep<B: Subst + ?Sized>(
    t: &Term,
    b: &B,
    run: &Truncation,
) -> Option<Term> {
    match t {
        Term::List(_) | Term::Triple(_) | Term::Formula(_) if compound_refused(t, b, run, true) => {
            None
        }
        _ => Some(apply_deep(t, b)),
    }
}

/// [`substitute_deep`] that also resolves inside the values it lands on
/// ([`apply_resolved`]), charged and refused for size and depth exactly as that one is.
pub(crate) fn substitute_resolved<B: Subst + ?Sized>(
    t: &Term,
    b: &B,
    run: &Truncation,
) -> Option<Term> {
    match t {
        Term::List(_) | Term::Triple(_) | Term::Formula(_) if compound_refused(t, b, run, true) => {
            None
        }
        _ => Some(apply_resolved(t, b, backward_depth(), &mut Vec::new())),
    }
}

/// Backward goal `g` with every variable resolved to the END of its binding chain in `b`
/// ([`super::walk`]), through list members, quoted triples AND quoted formulas. A
/// formula's OWN variables are not bound in `b`, so they are left as they are; what this
/// resolves inside one is a variable the CALLER bound, which the proof can read (a
/// `log:includes` premise walks into a quoted formula through the caller's binding, which
/// `backward_prove` hands the proof whole), so two goals that differ only there derive
/// different values and must not share a name.
///
/// This, and not [`substitute`], is what a goal's IDENTITY is built from. Substitution
/// follows ONE binding, while the unification that resolves a goal
/// ([`super::unify_walked`]) follows the whole chain, so two goals unification tells
/// apart can substitute alike: a repeated goal variable (`{ (?t ?t) :outer ?f }`) aliases
/// one renamed variable to another, and the two goals then differ only past that one
/// binding. Naming both with one identity merges their answers and loses a row
/// (GH #6844).
///
/// A variable whose expansion this is already INSIDE is left as it is: a formula bound to
/// `?x` may itself name `?x` (`:s :p { ?x :q ?x }`, where the query binds `?x` to that
/// formula), and that occurrence is the formula's OWN name, not the caller's binding of
/// it. Resolving it would walk back into the same formula at every occurrence, round and
/// round until the depth room ran out, which refused the goal and dropped the answer
/// (GH #6844 review round 5). An occurs check on the walk stack stops that at the first
/// re-entry, and it terminates for every binding: there are finitely many names, so an
/// endless walk would have to expand one of them twice.
///
/// `None`, with the cut recorded, past the run's term allowance: its depth limit bounds
/// the levels this recurses (as [`generated`] bounds what evaluation keeps) and each TERM
/// it builds is charged against its size limit ([`substituted_refused`]), which is a
/// per-term limit, so the three terms of an atom are counted apart as [`substitute`]
/// counts them. A goal that has no identity cannot be named, so `backward_prove` fails
/// the run on it rather than return an empty proof, which would read as "not provable".
///
/// The OWNING form, which is what this module's tests pin the walk through; the evaluation
/// itself goes through [`walked_atom_cow`], of which this is a clone of each term.
#[cfg(test)]
pub(crate) fn walked_atom(
    g: &[Term; 3],
    b: &super::Binding,
    run: &Truncation,
) -> Option<[Term; 3]> {
    walked_atom_cow(g, b, run).map(|ts| ts.map(std::borrow::Cow::into_owned))
}

/// [`walked_atom`] without copying the terms it did not have to build: each term comes back
/// BORROWED when the walk left it as it found it (a leaf, resolved or not — which is every
/// term of an ordinary goal) and OWNED only where a compound was rebuilt member by member.
///
/// [`walked_atom`] is this plus a clone of each, and is what a caller that needs the atom
/// itself uses. A caller that only READS the walked goal — building its canonical key, which
/// is all `backward_prove` wants it for — takes this instead, because cloning three terms
/// per goal of the proof search was a per-goal allocation GH #7095 added for nothing
/// (GH #6844 follow-up). One walk, one set of refusals: there is no second implementation of
/// the walk to drift from this one.
pub(crate) fn walked_atom_cow<'a>(
    g: &'a [Term; 3],
    b: &'a super::Binding,
    run: &Truncation,
) -> Option<[std::borrow::Cow<'a, Term>; 3]> {
    let room = run.term_depth.get();
    // One byte counter PER TERM: `max_term_bytes` bounds one term, so three terms that are
    // each admissible on their own must not refuse the atom by their sum. The occurs-check
    // stack is per term too: it holds the variables THIS term's walk is inside.
    let walk_term = |t: &'a Term| walked(t, b, run, room, &mut 0usize, &mut Vec::new());
    Some([walk_term(&g[0])?, walk_term(&g[1])?, walk_term(&g[2])?])
}

/// One term of [`walked_atom`], `room` levels of nesting and `spent` bytes of the atom's
/// allowance in, with `open` naming the variables whose expansion this walk is inside.
/// Private: callers go through [`walked_atom`].
fn walked<'a>(
    t: &'a Term,
    b: &'a super::Binding,
    run: &Truncation,
    room: usize,
    spent: &mut usize,
    open: &mut Vec<&'a str>,
) -> Option<std::borrow::Cow<'a, Term>> {
    // The occurs check: a variable this walk is already inside the expansion of is the
    // formula's own name and stays as it is.
    let own = matches!(t, Term::Var(v) if open.iter().any(|o| *o == v));
    // BORROWED ([`super::walk_ref`]): the resolved term is only read here, and a compound is
    // rebuilt member by member below, so cloning it first deep-copied a formula or a list
    // that was then thrown away (GH #6844 follow-up).
    let resolved: &Term = if own { t } else { super::walk_ref(t, b) };
    let compound = matches!(resolved, Term::List(_) | Term::Triple(_) | Term::Formula(_));
    // A compound is charged for its own node here and for its members as they resolve; a
    // leaf is charged for all of itself at once.
    let node = if compound { TERM_NODE_BYTES } else { term_bytes(resolved) };
    *spent = spent.saturating_add(node);
    if substituted_refused(run, *spent) {
        return None;
    }
    // Only a compound descends, so only a compound needs room to descend INTO, and only a
    // compound may take a level off `room`: a leaf resolved at the boundary (`room` 0) is
    // returned, not refused and not counted (which would underflow).
    if !compound {
        return Some(std::borrow::Cow::Borrowed(resolved));
    }
    if room == 0 {
        return settle(run, Bounded::cut(None, TERM_DEPTH_CUT));
    }
    // Descending into the value of a variable: that variable is OPEN for the walk below,
    // so an occurrence of it inside its own value is left alone by the check above. Popped
    // after, and a refusal below abandons the whole atom, so the stack is never read again.
    // `&str`, not an owned `String`: the name lives in `t`, which outlives this walk, so
    // the occurs-check stack no longer copies a variable name per descent.
    let expanded = match t {
        Term::Var(v) if !own => {
            open.push(v.as_str());
            true
        }
        _ => false,
    };
    let out = walked_members(resolved, b, run, room - 1, spent, open).map(std::borrow::Cow::Owned);
    if expanded {
        open.pop();
    }
    out
}

/// The members of compound `resolved`, each walked with `inner` levels of room left: the
/// descending half of [`walked`], split out so its caller can pop the occurs-check stack
/// on every path out of it.
fn walked_members<'a>(
    resolved: &'a Term,
    b: &'a super::Binding,
    run: &Truncation,
    inner: usize,
    spent: &mut usize,
    open: &mut Vec<&'a str>,
) -> Option<Term> {
    Some(match resolved {
        Term::List(ms) => {
            let mut out = Vec::with_capacity(ms.len());
            for m in ms {
                out.push(walked(m, b, run, inner, spent, open)?.into_owned());
            }
            Term::List(out)
        }
        Term::Triple(tr) => Term::Triple(Box::new([
            walked(&tr[0], b, run, inner, spent, open)?.into_owned(),
            walked(&tr[1], b, run, inner, spent, open)?.into_owned(),
            walked(&tr[2], b, run, inner, spent, open)?.into_owned(),
        ])),
        Term::Formula(ts) => {
            let mut out = Vec::with_capacity(ts.len());
            for r in ts {
                out.push([
                    walked(&r[0], b, run, inner, spent, open)?.into_owned(),
                    walked(&r[1], b, run, inner, spent, open)?.into_owned(),
                    walked(&r[2], b, run, inner, spent, open)?.into_owned(),
                ]);
            }
            Term::Formula(out)
        }
        // Not reachable: only a compound descends here. Cloned rather than moved now that
        // the compound is borrowed, which costs nothing on a path nothing takes.
        other => other.clone(),
    })
}

/// Whether compound `t`, substituted under `b`, is past the run's term-size limit
/// (sized from the borrowed terms; the cut and the run's refusal are recorded). Kept out
/// of line: a variable or atom, the common case, never reaches it.
#[inline(never)]
fn compound_refused<B: Subst + ?Sized>(
    t: &Term,
    b: &B,
    run: &Truncation,
    deep: bool,
) -> bool {
    substituted_refused(run, resolved_bytes(t, b, deep))
}

/// Whether a term built under a rule's bindings, of `bytes` ([`term_bytes`], computed
/// without building it), is past the run's term-size limit: the cut and the run's refusal
/// are recorded. The compiled evaluator sizes a builtin's argument list through it, as
/// [`substitute`] sizes the list the text engine builds.
pub(crate) fn substituted_refused(run: &Truncation, bytes: usize) -> bool {
    settle(run, derived_term(run, &Substituted, bytes, || bytes))
}

/// The [`term_bytes`] of dictionary term `id`, as the text engine's term for it would
/// count, read from the dictionary's borrowed records without building the term (an
/// inline integer is counted from its digits; a triple term walks its components with an
/// explicit stack).
#[cfg(feature = "compiled-rules")]
pub(crate) fn dict_term_bytes(dict: &sparq_core::dict::Dict, id: sparq_core::dict::Id) -> usize {
    use sparq_core::dict::{is_inline, TermParts, INLINE_BASE};
    const XSD_INTEGER_LEN: usize = "http://www.w3.org/2001/XMLSchema#integer".len();
    // One node's own bytes, or its components when it is a triple term.
    let node = |id| {
        if is_inline(id) {
            return Ok(super::digit_count(u128::from(id - INLINE_BASE)) + XSD_INTEGER_LEN);
        }
        Ok(match dict.term_parts(id) {
            TermParts::Iri { prefix, suffix } => prefix.len().saturating_add(suffix.len()),
            TermParts::Lit { value, datatype, lang } => value
                .len()
                .saturating_add(datatype.len())
                .saturating_add(lang.map_or(0, str::len)),
            TermParts::Blank(b) => b.len(),
            TermParts::Triple(ids) => return Err(ids),
        })
    };
    let ids = match node(id) {
        Ok(n) => return n.saturating_add(TERM_NODE_BYTES),
        Err(ids) => ids,
    };
    let mut total = TERM_NODE_BYTES;
    let mut stack = ids.to_vec();
    while let Some(id) = stack.pop() {
        total = total.saturating_add(TERM_NODE_BYTES);
        match node(id) {
            Ok(n) => total = total.saturating_add(n),
            Err(ids) => stack.extend(ids),
        }
    }
    total
}

/// Substitute bound variables in `t` (recursing into list members and quoted triples);
/// returns the term (possibly still containing free vars). Private: callers go through
/// [`substitute`].
fn apply<B: Subst + ?Sized>(t: &Term, b: &B) -> Term {
    match t {
        Term::Var(v) => b.bound(v).cloned().unwrap_or_else(|| t.clone()),
        Term::List(ms) => Term::List(ms.iter().map(|m| apply(m, b)).collect()),
        Term::Triple(tr) => {
            Term::Triple(Box::new([apply(&tr[0], b), apply(&tr[1], b), apply(&tr[2], b)]))
        }
        _ => t.clone(),
    }
}

/// [`apply`] through quoted formulae too. Private: callers go through
/// [`substitute_deep`].
fn apply_deep<B: Subst + ?Sized>(t: &Term, b: &B) -> Term {
    match t {
        Term::Var(v) => match b.bound(v) {
            Some(val) => val.clone(),
            None => t.clone(),
        },
        Term::List(ms) => Term::List(ms.iter().map(|m| apply_deep(m, b)).collect()),
        Term::Formula(ts) => Term::Formula(
            ts.iter()
                .map(|r| [apply_deep(&r[0], b), apply_deep(&r[1], b), apply_deep(&r[2], b)])
                .collect(),
        ),
        Term::Triple(tr) => Term::Triple(Box::new([
            apply_deep(&tr[0], b),
            apply_deep(&tr[1], b),
            apply_deep(&tr[2], b),
        ])),
        _ => t.clone(),
    }
}

/// [`apply_deep`], and then INTO whatever each variable landed on.
///
/// A term a substitution binds a variable to can itself hold variables the same substitution
/// binds. [`apply_deep`] clones such a value and stops, which left them behind:
/// `{ ?s :pat { ?l :p :b } } <= { ( { ?y :q :r } ) list:member ?l . ?s :val ?y }` returned
/// `{ { ?__bw<n>_y :q :r } :p :b }` with `?y` bound to its value all along (GH #6943).
///
/// This is NOT what ordinary substitution should do — a quoted formula is a value, and a
/// formula a caller passes in has to come back as the caller wrote it, which is why
/// [`apply_deep`] stops where it does. Only two callers want the resolving form: the
/// projection of a backward proof onto its goal, which is instantiating what the proof itself
/// produced, and the builtins that must match an arriving pattern through the same resolver
/// their groundness gate used. Both go through [`substitute_resolved`].
///
/// `fuel` bounds the descent, and `open` is the cycle guard: it holds the values currently
/// being resolved into, by ADDRESS in the substitution, and a value already open is cloned
/// instead of re-entered. Without it a self-referential binding does terminate on `fuel`,
/// but it UNROLLS first — a variable bound to a formula mentioning itself came back as that
/// formula nested as deep as the fuel allowed, where [`apply_deep`] returned it once. The
/// guard restores that: on a cycle the variable is left unresolved, which is the term
/// [`apply_deep`] produces for the same binding.
/// Does `t` mention a variable named `v`, at any depth?
fn names_var(t: &Term, v: &str) -> bool {
    match t {
        Term::Var(n) => n.as_str() == v,
        Term::List(ms) => ms.iter().any(|m| names_var(m, v)),
        Term::Formula(ts) => ts.iter().flatten().any(|m| names_var(m, v)),
        Term::Triple(tr) => tr.iter().any(|m| names_var(m, v)),
        _ => false,
    }
}

fn apply_resolved<B: Subst + ?Sized>(
    t: &Term,
    b: &B,
    fuel: usize,
    open: &mut Vec<*const Term>,
) -> Term {
    if fuel == 0 {
        return t.clone();
    }
    match t {
        Term::Var(v) => match b.bound(v) {
            Some(val) => {
                // Occurs check: a value that mentions the very variable it is bound to is an
                // infinite term, and expanding it once more only renders one more level of
                // it. Take it as it stands, which is the term [`apply_deep`] produces.
                if names_var(val, v) {
                    return val.clone();
                }
                let at: *const Term = val;
                if open.iter().any(|o| std::ptr::eq(*o, at)) {
                    // Already being resolved further up: this is a cycle through the
                    // substitution. Leave the VARIABLE as it stands rather than expanding
                    // the value once more, which is what [`apply_deep`] does and so keeps a
                    // cyclic binding rendering exactly as it did before.
                    return t.clone();
                }
                open.push(at);
                let out = apply_resolved(val, b, fuel - 1, open);
                open.pop();
                out
            }
            None => t.clone(),
        },
        Term::List(ms) => {
            Term::List(ms.iter().map(|m| apply_resolved(m, b, fuel - 1, open)).collect())
        }
        Term::Formula(ts) => Term::Formula(
            ts.iter()
                .map(|r| {
                    [
                        apply_resolved(&r[0], b, fuel - 1, open),
                        apply_resolved(&r[1], b, fuel - 1, open),
                        apply_resolved(&r[2], b, fuel - 1, open),
                    ]
                })
                .collect(),
        ),
        Term::Triple(tr) => Term::Triple(Box::new([
            apply_resolved(&tr[0], b, fuel - 1, open),
            apply_resolved(&tr[1], b, fuel - 1, open),
            apply_resolved(&tr[2], b, fuel - 1, open),
        ])),
        _ => t.clone(),
    }
}

/// The round and fact allowance of one forward closure (`run_closure`).
pub(crate) struct ClosureRounds {
    /// Rounds the closure may still run.
    left: usize,
    /// The allowance it started with.
    limits: ClosureLimits,
}

impl ClosureRounds {
    /// The allowance of a fresh forward closure of the run `run`: its [`ClosureLimits`].
    pub(crate) fn forward(run: &Truncation) -> Self {
        #[cfg(test)]
        let limits = tests::CLOSURE_OVERRIDE.with(Cell::get).map_or(
            run.limits,
            |(max_rounds, max_facts)| ClosureLimits {
                max_rounds,
                max_facts,
                ..run.limits
            },
        );
        #[cfg(not(test))]
        let limits = run.limits;
        ClosureRounds {
            left: limits.max_rounds,
            limits,
        }
    }

    /// One more round, the closure now holding `facts` facts. Past the allowance the
    /// closure is incomplete: the cut is recorded on `cuts` (so every enclosing run's
    /// negation gate refuses to read it) and the closure fails with an error, so no
    /// caller takes the truncated fact set for a fixpoint.
    pub(crate) fn round(&mut self, facts: usize, cuts: &impl Sink) -> Result<(), String> {
        match self.left.checked_sub(1) {
            Some(left) if facts <= self.limits.max_facts => {
                self.left = left;
                Ok(())
            }
            _ => Err(self.exceeded(cuts)),
        }
    }

    /// The fact allowance alone, without spending a round: `Err` (the cut recorded on
    /// `cuts`) when a closure holding `facts` facts is past it. The incremental counting
    /// engine checks each new fact against it as it is collected, so no wave holds more
    /// distinct facts than the allowance before it is refused.
    pub(crate) fn hold(&self, facts: usize, cuts: &impl Sink) -> Result<(), String> {
        if facts <= self.limits.max_facts {
            return Ok(());
        }
        Err(self.exceeded(cuts))
    }

    /// The error of a closure past its allowance, the cut recorded on `cuts`.
    fn exceeded(&self, cuts: &impl Sink) -> String {
        cuts.record(CLOSURE_CUT);
        format!(
            "n3: {CLOSURE_CUT} ({} rounds, {} facts) before reaching a fixpoint: \
             the rules keep deriving new facts (raise ClosureLimits to allow more)",
            self.limits.max_rounds, self.limits.max_facts
        )
    }
}

/// Default [`runtime_docs`]: distinct documents (by text and base) one runtime-document
/// table may take.
const RUNTIME_DOCS: usize = 100_000;

/// Why a run was stopped by [`runtime_docs`].
const RUNTIME_DOCS_CUT: &str = "runtime-document limit exceeded";

/// Distinct documents (by text and base) one runtime-document table may take: a run's
/// own table, or a `MaterializedN3Graph`'s, which lives as long as the graph. The table
/// keeps a constant-size entry per document (its SHA-256 digest and label prefix, never
/// its text), so this bounds its memory; one more document fails the run
/// ([`runtime_docs_exceeded`]).
pub(crate) fn runtime_docs() -> usize {
    #[cfg(test)]
    if let Some(n) = RUNTIME_DOCS_OVERRIDE.with(Cell::get) {
        return n;
    }
    RUNTIME_DOCS
}

/// Why a run was stopped by [`proof_nodes`].
const PROOF_NODES_CUT: &str = "backward proof-node limit exceeded";

/// Distinct backward proof-tree nodes, and distinct string identities their keys are built
/// from, that one top-level run may register (`run_shared::RunShared::node` /
/// `intern`). The run keeps a constant-size entry per node, and a goal's or a nested
/// formula's canonical key per identity, so this bounds those tables; the one that would be
/// too many fails the run ([`proof_nodes_exceeded`]).
///
/// It is **[`ClosureLimits::max_join_rows`] × [`ClosureLimits::max_rounds`]**, with the
/// units stated honestly: `max_join_rows` bounds ONE ROUND's rows and firings, while these
/// tables accumulate over every round of the run, so the run-wide figure comparable to them
/// is the per-round allowance times the rounds the run may take. Charging the per-round
/// number alone conflated the two and refused a program a default-limits run answers (a
/// document with many `<=` sites and a few hundred subjects reaches a few thousand
/// registrations). No new field on `ClosureLimits`, which is public and built with struct
/// literals.
///
/// `usize::MAX` — the DEFAULT `max_join_rows` — saturates the product, so a default-limits
/// run is never refused for this, exactly as [`JoinWork`] is unbounded by default. Only a
/// run that set `max_join_rows` can be refused here, and the CHANGELOG and
/// `skills/inference/SKILL.md` say so.
///
/// It is not a charge against the [`JoinWork`] counter itself, because that one is
/// refreshed at the start of every round ([`JoinWork::round`]) and so cannot bound tables
/// that accumulate across all of them. A document with no `<=` rule registers nothing at
/// all, so this is never reached on the forward-only path. Alongside the allowance, a
/// `(rule, conclusion)` site is registered only once its conclusion UNIFIES with the goal
/// (`backward_prove`), so a site rejected at unification costs nothing — which is a smaller
/// saving than it sounds: a site whose conclusion carries a constant where the goal has a
/// free variable DOES unify, and it is the premise that then fails, so a document with many
/// such sites still registers one node per site per goal.
pub(crate) fn proof_nodes(run: &Truncation) -> usize {
    run.limits.max_join_rows.saturating_mul(run.limits.max_rounds.max(1))
}

/// The error of a run that has registered as many proof-tree nodes and identities as it
/// may, the cut recorded on `cuts`.
pub(crate) fn proof_nodes_exceeded(cuts: &impl Sink) -> String {
    cuts.record(PROOF_NODES_CUT);
    "n3: ".to_string()
        + PROOF_NODES_CUT
        + ": a run standardizes apart at most max_join_rows distinct backward proof-tree \
           nodes and goal identities (raise ClosureLimits::max_join_rows to allow more)"
}

/// The error of a run whose runtime-document table is full, the cut recorded on `cuts`.
pub(crate) fn runtime_docs_exceeded(cuts: &impl Sink) -> String {
    cuts.record(RUNTIME_DOCS_CUT);
    format!(
        "n3: {RUNTIME_DOCS_CUT}: a run (or a MaterializedN3Graph over its life) parses at \
         most {} distinct documents at run time",
        runtime_docs()
    )
}

#[cfg(test)]
std::thread_local! {
    /// A lowered [`runtime_docs`] for tables made on this thread.
    static RUNTIME_DOCS_OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Lowers [`runtime_docs`] on this thread until dropped.
#[cfg(test)]
pub(crate) struct RuntimeDocsOverride;

#[cfg(test)]
impl RuntimeDocsOverride {
    pub(crate) fn set(n: usize) -> RuntimeDocsOverride {
        RUNTIME_DOCS_OVERRIDE.with(|c| c.set(Some(n)));
        RuntimeDocsOverride
    }
}

#[cfg(test)]
impl Drop for RuntimeDocsOverride {
    fn drop(&mut self) {
        RUNTIME_DOCS_OVERRIDE.with(|c| c.set(None));
    }
}

/// Bracket nesting the N3 parser accepts: `{ }` formulae, `( )` lists, `[ ]` blank-node
/// property lists and `<< >>` quoted triples, counted together. The recursive descent
/// recurses once per level, so this is sized for the smallest stack the parser supports:
/// an ordinary 2 MiB thread in a debug build, where each level costs the most. It
/// leaves room for the caller's own frames, including a reasoning run that parses a
/// document with `log:parsedAsN3` / `log:semantics`, and every walker over the parsed
/// terms (matching, instantiation, interning, serialization) recurses less per level
/// than the parser. Pinned on a 2 MiB thread for every production, parsing and
/// reasoning, by the `parse_surface_tests` and `n3::tests` small-stack tests (#6732).
const PARSE_DEPTH: usize = 64;

/// Nesting (list, formula and quoted-triple levels) a term built during evaluation may
/// reach. A rule can wrap a term in one more level each round (`{ :s :p ?x } => { :s :p
/// (?x) }`), and a chain of premise atoms can within one firing, so without a limit a
/// shallow document builds terms deep enough that substitution, matching, hashing and
/// dropping them exhaust the stack. A term evaluation builds past this is dropped as a
/// cut ([`generated`]); everything evaluation keeps is at most this deep, and what it
/// builds before the check is at most this plus the nesting of the rule text. A run
/// handed deeper terms directly (as terms, not text: [`given_terms`]) keeps terms as
/// deep as those, and no deeper. Twice [`PARSE_DEPTH`], so a parsed document
/// (`log:semantics`, `log:parsedAsN3`) quoted as a formula value, and rules over it,
/// stay inside. Pinned on a 2 MiB thread by the `n3::tests` small-stack tests (#6732).
const TERM_DEPTH: usize = 2 * PARSE_DEPTH;

/// Why a generated term was dropped.
const TERM_DEPTH_CUT: &str = "a term built during evaluation passed the term-depth limit";

/// Whether `t` nests at most `room` levels deep. It recurses at most `room + 1` levels,
/// however deep `t` is.
fn fits(t: &super::Term, room: usize) -> bool {
    use super::Term;
    match t {
        Term::List(ms) => room > 0 && ms.iter().all(|m| fits(m, room - 1)),
        Term::Formula(ts) => room > 0 && ts.iter().flatten().all(|m| fits(m, room - 1)),
        Term::Triple(tr) => room > 0 && tr.iter().all(|m| fits(m, room - 1)),
        Term::Iri(_) | Term::Lit(..) | Term::Blank(_) | Term::Var(_) => true,
    }
}

/// The nesting of `t` (0 for an IRI, literal, blank node or variable; one more for each
/// list, formula or quoted triple around it, an empty one included), walked with an
/// explicit stack, so it takes no stack per level. It agrees with [`fits`]: `fits(t,
/// room)` holds exactly when `depth(t) <= room`.
fn depth(t: &super::Term) -> usize {
    use super::Term;
    let mut deepest = 0;
    // Each entry: a term, and how many containers enclose it.
    let mut stack: Vec<(&Term, usize)> = vec![(t, 0)];
    while let Some((t, d)) = stack.pop() {
        let below = d + 1;
        match t {
            Term::List(ms) => stack.extend(ms.iter().map(|m| (m, below))),
            Term::Formula(ts) => stack.extend(ts.iter().flatten().map(|m| (m, below))),
            Term::Triple(tr) => stack.extend(tr.iter().map(|m| (m, below))),
            Term::Iri(_) | Term::Lit(..) | Term::Blank(_) | Term::Var(_) => continue,
        }
        deepest = deepest.max(below);
    }
    deepest
}

/// The terms a TOP-LEVEL run was handed (its facts and rules). Parsed text is within
/// [`PARSE_DEPTH`], but a caller can hand a run terms built in code (the incremental
/// graph's base facts): a rule that copies one builds nothing deeper than its input, so
/// the run's term allowance ([`generated`]) is raised to the deepest of them. Rules still
/// cannot build past it.
pub(crate) fn given_terms<'a>(run: &Truncation, terms: impl IntoIterator<Item = &'a super::Term>) {
    let room = run.term_depth.get();
    // Only a term that does not fit already is measured.
    let deepest = terms
        .into_iter()
        .filter(|t| !fits(t, room))
        .map(depth)
        .max()
        .unwrap_or(0);
    if deepest > run.term_depth.get() {
        run.term_depth.set(deepest);
    }
}

/// Terms evaluation built (a conclusion, the values of a binding) in the run `run`:
/// `true` when every one is within the run's term allowance ([`TERM_DEPTH`], or the
/// deepest term the run was given); otherwise `false` and a cut, and the caller drops
/// them.
pub(crate) fn generated<'a>(
    run: &Truncation,
    terms: impl IntoIterator<Item = &'a super::Term>,
) -> Bounded<bool> {
    let room = run.term_depth.get();
    if terms.into_iter().all(|t| fits(t, room)) {
        Bounded::complete(true)
    } else {
        Bounded::cut(false, TERM_DEPTH_CUT)
    }
}

/// Stack one run's recursive evaluation may hold at once, in units of one backward
/// (`<=`) proof step (the heaviest recursion per level). A backward proof step and a
/// scoped operator's inner clause matched against the store ([`nested_step`]) each hold
/// one unit while they run, and a nested forward closure ([`CLOSURE_FRAMES`]) holds its
/// units while it runs. Every nested closure of the run draws on the same allowance, so
/// a chain of distinct documents (`log:semantics` then `log:conclusion`, each closing the
/// next) cannot reset it, and neither can a fresh backward search inside one. It
/// is sized for the smallest stack the engine supports (a 2 MiB thread in a debug build),
/// leaving room for a parse up to [`PARSE_DEPTH`] deep and the term walkers at the
/// innermost level. It admits a top-level backward search of [`BW_DEPTH`] steps with room
/// to spare, so it binds only nested evaluation. Pinned on a 2 MiB thread by the
/// `n3::tests` small-stack tests (#6732).
const RUN_STACK: usize = 128;
/// The units of [`RUN_STACK`] one nested forward closure holds: its closure's own frames
/// and the builtin evaluation that entered it.
const CLOSURE_FRAMES: usize = 8;

/// Why a nested closure was refused.
const CLOSURE_NESTING_CUT: &str = "nested closures passed the run's stack allowance";

/// Units of the run's stack allowance ([`RUN_STACK`]) held by a step in progress; dropping
/// it gives them back.
pub(crate) struct StackFrames {
    held: Rc<Cell<usize>>,
    units: usize,
}

impl Drop for StackFrames {
    fn drop(&mut self) {
        self.held.set(self.held.get().saturating_sub(self.units));
    }
}

/// Take `units` of the run's stack allowance, or `None` when that would pass it.
fn take_stack(run: &Truncation, units: usize) -> Option<StackFrames> {
    let held = run.stack.get() + units;
    (held <= RUN_STACK).then(|| {
        run.stack.set(held);
        StackFrames {
            held: Rc::clone(&run.stack),
            units,
        }
    })
}

/// One more nested premise evaluation of the run `run` (the inner clause of
/// `log:notIncludes`, `log:collectAllIn`, `log:forAllIn` or a store-scoped
/// `log:includes`, matched against the store), holding one unit of the run's stack
/// allowance until dropped. `None` when it is spent, which is a cut: the clause is not
/// evaluated, so a negation or aggregation over it is then refused.
pub(crate) fn nested_step(run: &Truncation) -> Bounded<Option<StackFrames>> {
    match take_stack(run, 1) {
        Some(frames) => Bounded::complete(Some(frames)),
        None => Bounded::cut(
            None,
            "a scoped clause nested past the run's stack allowance",
        ),
    }
}

/// One more nested forward closure of the run `run` (`log:conclusion`,
/// `log:supports`), holding its units of the run's stack allowance until dropped. Past
/// the allowance the closure cannot be computed: the cut is recorded on `run` and the
/// closure fails with an error, which refuses the enclosing run.
pub(crate) fn enter_closure(run: &Truncation) -> Result<StackFrames, String> {
    take_stack(run, CLOSURE_FRAMES).ok_or_else(|| {
        run.record(CLOSURE_NESTING_CUT);
        format!(
            "n3: {CLOSURE_NESTING_CUT}: log:conclusion / log:supports closures nested too \
             deep (each nested closure, and each backward proof step, holds part of a \
             stack allowance of {RUN_STACK} units)"
        )
    })
}

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

/// The deepest nesting the parser accepts.
#[cfg(test)]
pub(crate) const fn parse_depth() -> usize {
    PARSE_DEPTH
}

/// The deepest term evaluation keeps.
#[cfg(test)]
pub(crate) const fn term_depth() -> usize {
    TERM_DEPTH
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

/// As [`parse_n3_with_extra`], with `facts` in place of the document's own facts
/// ([`super::parser::parse_rules_with_facts_checked`]).
pub(crate) fn parse_n3_rules_with_facts(
    src: &str,
    base: &str,
    facts: impl IntoIterator<Item = [super::Term; 3]>,
    cuts: &impl Sink,
) -> Result<super::parser::Parsed, String> {
    use super::parser::{parse_rules_with_facts_checked, ParseFailure};
    let parsed = match parse_rules_with_facts_checked(src, base, facts) {
        Ok(p) => Bounded::complete(Ok(p)),
        Err(ParseFailure::Syntax(m)) => Bounded::complete(Err(m)),
        Err(ParseFailure::Resource(e)) => Bounded::cut(Err(e.message), e.why),
    };
    settle(cuts, parsed)
}

/// Why a builtin's parse was stopped by [`ClosureLimits::max_term_bytes`].
const PARSE_TERM_CUT: &str = "forward closure limit exceeded (a parsed formula)";

/// Charge `bytes` to a charged parse's allowance with `left` bytes left: what is left
/// after it, or the limit error once the parse would pass it.
pub(crate) fn charge_parse(left: usize, bytes: usize) -> Result<usize, LimitError> {
    left.checked_sub(bytes).ok_or_else(|| LimitError {
        why: PARSE_TERM_CUT,
        message: format!(
            "n3: {CLOSURE_CUT}: a parsed formula would take more than max_term_bytes \
             (raise ClosureLimits::max_term_bytes to allow more)"
        ),
    })
}

/// Parse N3 `src` into a term a builtin derives (`log:parsedAsN3`, `log:semantics`) under
/// the run's [`ClosureLimits::max_term_bytes`]: the parser charges every byte it keeps
/// before allocating it (each string, term node and copy; `parse_quoted_charged_checked`,
/// which reads the document's own `@forAll` as the formula's), so a
/// small source whose prefixes, base or shared formulas expand into a huge formula is
/// refused while it is parsed. On that refusal the run is refused
/// (a cut, and an error every closure of the run fails with). `None` for that and for any
/// other failure: a syntax error, or the nesting limit (itself recorded as a cut).
pub(crate) fn parse_n3_term(
    src: &str,
    base: &str,
    run: &Truncation,
    cuts: &impl Sink,
) -> Option<super::parser::Parsed> {
    use super::parser::{parse_quoted_charged_checked, ParseFailure};
    let allowance = Some(run.limits.max_term_bytes);
    match parse_quoted_charged_checked(src, base, allowance) {
        Ok(p) => Some(p),
        Err(ParseFailure::Syntax(_)) => None,
        Err(ParseFailure::Resource(e)) => {
            cuts.record(e.why);
            if e.why == PARSE_TERM_CUT {
                run.refuse(|| e.message);
            }
            None
        }
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

    /// #6953: a formula match's free allowance covers only that match: what it leaves
    /// unused is dropped when it ends, so repeated matches never top up the budget.
    #[test]
    fn free_formula_steps_never_top_up_the_budget() {
        let mut steps = StepBudget::unification();
        for _ in 0..3 {
            steps.enter_formula(UNIFICATION_BUDGET);
            steps.enter_formula(UNIFICATION_BUDGET); // nested: no allowance of its own
            assert!(steps.take());
            steps.exit_formula();
            steps.exit_formula();
        }
        let taken = std::iter::from_fn(|| steps.take().then_some(())).count();
        assert_eq!(taken, UNIFICATION_BUDGET);
        steps.enter_formula(10);
        assert!(!steps.take(), "a spent budget stays spent");
    }

    #[test]
    fn the_unbounded_join_work_never_refuses() {
        let run = Truncation::top_level();
        let work = JoinWork::forward(&run);
        assert!(work.charge(usize::MAX, &run));
        assert!(work.charge(usize::MAX, &run));
        assert!(work.charge(usize::MAX.saturating_mul(3), &run));
        work.round();
        assert!(work.charge(usize::MAX, &run));
        assert_eq!(run.refusal(), None);
        assert_eq!(run.get(), None);
        // A set limit is checked exactly.
        let run = Truncation::top_level_with(ClosureLimits {
            max_join_rows: 2,
            ..ClosureLimits::default()
        });
        let work = JoinWork::forward(&run);
        assert!(work.charge(2, &run));
        assert!(!work.charge(1, &run));
        assert!(run.refusal().is_some_and(|e| e.contains("max_join_rows (2)")));
    }

    thread_local! {
        /// A test's `(rounds, facts)` closure allowance in place of the defaults.
        pub(super) static CLOSURE_OVERRIDE: Cell<Option<(usize, usize)>> =
            const { Cell::new(None) };
    }

    fn cut_of<T>(b: Bounded<T>) -> (T, bool) {
        let sink = Cell::new(false);
        let v = settle(&sink, b);
        (v, sink.get())
    }

    /// The GH #6757 shape, still diverging: a backward rule concludes a formula holding a
    /// variable (or an `@forAll` universal), a forward rule nests that value one level
    /// deeper, and a second forward rule feeds the deeper value back to the backward rule's
    /// own predicate. Each round's value is one `{ :s :t … }` deeper than the last, so the
    /// closure never saturates.
    ///
    /// The feedback is what diverges. The RENAMING no longer does: a `__bw…` name is the
    /// fingerprint of the proof-tree node and the GOAL, so re-proving the same goal at the
    /// same node in a later round mints the same name (GH #6844) — the four programs that
    /// diverged on the counter alone now saturate, which
    /// `the_gh_6757_renaming_programs_saturate` pins.
    const DIVERGING: [&str; 4] = [
        "@prefix : <http://example.org/#>.\n\
         { :a :p { ?x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r { :s :t ?f } } .\n\
         { :a :r ?f } => { :a :p ?f } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r { :s :t ?f } } .\n\
         { :a :r ?f } => { :a :p ?f } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :b :r { :s :t ?f } } .\n\
         { :b :r ?f } => { :a :p ?f } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r { :s :t { :s :u ?f } } } .\n\
         { :a :r ?f } => { :a :p ?f } .\n",
    ];

    /// The four GH #6757 programs AS THEY WERE WRITTEN THERE: a backward rule concludes a
    /// formula holding a variable (or an `@forAll` universal) and a forward rule copies it
    /// out, with nothing feeding the copy back. Each round re-proves the same goal, so the
    /// only thing that could differ between rounds was the freshened variable's name.
    const GH_6757_RENAMING_ONLY: [&str; 4] = [
        "@prefix : <http://example.org/#>.\n\
         { :a :p { ?x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r ?f } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :a :r { :s :t ?f } } .\n",
        "@prefix : <http://example.org/#>.\n\
         @forAll :x .\n\
         { :a :p { :x :q :z } } <= true .\n\
         { :a :p ?f } => { :b :r ?f } .\n",
    ];

    /// `f` under a small closure allowance.
    fn with_allowance<T>(rounds: usize, facts: usize, f: impl FnOnce() -> T) -> T {
        CLOSURE_OVERRIDE.with(|c| c.set(Some((rounds, facts))));
        let out = f();
        CLOSURE_OVERRIDE.with(|c| c.set(None));
        out
    }

    fn assert_closure_limit<T: std::fmt::Debug>(what: &str, r: Result<T, String>) {
        match r {
            Err(e) => assert!(e.contains(CLOSURE_CUT), "{what}: {e}"),
            Ok(v) => panic!("{what}: a truncated closure came back as a result: {v:?}"),
        }
    }

    /// Every public entry point fails with the closure-limit error on a diverging
    /// program, instead of returning the truncated closure or running out of memory.
    fn every_entry_point_refuses(doc: &str, rounds: usize, facts: usize) {
        use crate::n3::{self, RuleVars};
        use sparq_core::dict::Dict;
        with_allowance(rounds, facts, || {
            assert_closure_limit("reason_n3", n3::reason_n3(&mut Dict::new(), doc));
            assert_closure_limit(
                "reason_n3_terms",
                n3::reason_n3_terms(doc, None).map(|c| c.facts.len()),
            );
            assert_closure_limit(
                "reason_n3_proof",
                n3::reason_n3_proof(&mut Dict::new(), doc),
            );
            assert_closure_limit(
                "reason_n3_proof_run",
                n3::reason_n3_proof_run(&mut Dict::new(), doc).map(|r| r.closure),
            );
            for vars in [RuleVars::N3, RuleVars::VarIris] {
                assert_closure_limit("reason_n3_pass_all", n3::reason_n3_pass_all(doc, vars));
            }
            let query = "@prefix : <http://example.org/#>. { :a :r ?f } => { :a :answer ?f } .";
            assert_closure_limit(
                "reason_n3_query_terms",
                n3::reason_n3_query_terms(doc, query),
            );
            assert_closure_limit(
                "reason_n3_query",
                n3::reason_n3_query(&mut Dict::new(), doc, query),
            );
            assert_closure_limit(
                "reason_n3_stratified",
                n3::reason_n3_stratified(&mut Dict::new(), &["", doc]).map(|c| c.facts),
            );
            // The incremental graph's batch fallback refuses the rules the same way.
            assert_closure_limit(
                "MaterializedN3Graph::new",
                crate::MaterializedN3Graph::new(doc, &[]).map(|g| g.closure().len()),
            );
        });
    }

    #[test]
    fn a_diverging_closure_stops_at_its_limit_with_an_error() {
        for doc in DIVERGING {
            every_entry_point_refuses(doc, 64, CLOSURE_FACTS);
            // The fact allowance trips on its own too.
            every_entry_point_refuses(doc, CLOSURE_ROUNDS, 32);
        }
    }

    /// GH #6844 item 2: the backward chainer's freshened names no longer count
    /// applications, so re-proving one goal in a later round derives the SAME fact and the
    /// closure saturates. These four programs were refused at the closure limit before.
    #[test]
    fn the_gh_6757_renaming_programs_saturate() {
        use crate::n3;
        for doc in GH_6757_RENAMING_ONLY {
            // Far below the old allowance: the closure is one fact, reached in a few rounds.
            let closure = with_allowance(16, 8, || n3::reason_n3_terms(doc, None))
                .unwrap_or_else(|e| panic!("still diverges: {e}\n{doc}"));
            let derived: Vec<&[Term; 3]> = closure
                .facts
                .iter()
                .filter(|[_, p, _]| matches!(p, Term::Iri(i) if i.ends_with("#r")))
                .collect();
            assert_eq!(derived.len(), 1, "{doc}: {derived:?}");
        }
    }

    #[test]
    fn a_diverging_closure_with_negation_is_refused() {
        for doc in DIVERGING {
            let neg = format!(
                "{doc}@prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
                 {{ ?s :r ?f. ?s log:notIncludes {{ ?s :never ?f }} }} => {{ ?s :checked ?f }} .\n\
                 {{ :a :r ?f. (?f {{ :a :r ?f }} ?all) log:collectAllIn _:t. }} => {{ :a :all ?all }} .\n"
            );
            every_entry_point_refuses(&neg, 64, CLOSURE_FACTS);
        }
    }

    #[test]
    fn a_terminating_program_reports_no_cut_under_the_default_allowance() {
        use crate::n3;
        // A transitive chain and a negation over its closure: hundreds of rounds, no cut.
        let mut doc = String::from(
            "@prefix : <http://example.org/#>.\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
             { ?x :p ?y. ?y :p ?z } => { ?x :p ?z } .\n\
             { :n0 :p ?y. ?scope log:notIncludes { ?y :p :n0 } } => { ?y :reachedFrom :n0 } .\n",
        );
        for i in 0..300 {
            doc.push_str(&format!(":n{i} :p :n{} .\n", i + 1));
        }
        let closure = n3::reason_n3_terms(&doc, None).expect("a terminating program closes");
        assert!(
            closure.facts.len() > 300 * 301 / 2,
            "{}",
            closure.facts.len()
        );
        let mut allowance = ClosureRounds::forward(&Truncation::top_level());
        assert_eq!(allowance.limits, ClosureLimits::default());
        assert_eq!(
            ClosureLimits::default(),
            ClosureLimits {
                max_rounds: 1_000_000,
                max_facts: 50_000_000,
                max_term_bytes: 16 << 20,
                max_join_rows: usize::MAX,
                max_total_join_rows: usize::MAX,
            }
        );
        let sink = Cell::new(false);
        assert!(allowance.round(closure.facts.len(), &sink).is_ok());
        assert!(!sink.get());
    }

    #[test]
    fn the_closure_allowance_records_a_cut_when_spent() {
        let small = ClosureLimits {
            max_rounds: 2,
            max_facts: 10,
            ..ClosureLimits::default()
        };
        let sink = Truncation::top_level_with(small);
        let mut allowance = ClosureRounds::forward(&sink);
        assert!(allowance.round(10, &sink).is_ok());
        assert!(allowance.round(10, &sink).is_ok());
        assert!(sink.get().is_none());
        assert!(allowance.round(10, &sink).is_err());
        assert_eq!(sink.get(), Some(CLOSURE_CUT));
        let fresh = Truncation::top_level_with(small);
        let mut allowance = ClosureRounds::forward(&fresh);
        assert!(allowance.round(11, &fresh).is_err());
        assert_eq!(fresh.get(), Some(CLOSURE_CUT));
    }

    /// A finite linear walk: `:start :reaches` each node of an `n`-edge `:next` chain.
    /// It closes in about `n` rounds and holds about `2n` facts.
    fn linear_chain(n: usize) -> String {
        let mut doc = String::from(
            "@prefix : <http://example.org/#>.\n\
             :start :reaches :n0 .\n\
             { :start :reaches ?x. ?x :next ?y } => { :start :reaches ?y } .\n",
        );
        for i in 0..n {
            doc.push_str(&format!(":n{i} :next :n{} .\n", i + 1));
        }
        doc
    }

    /// The closure of `doc` under `limits`: its fact count, or the error, and the cut.
    fn close_under(
        doc: &str,
        limits: ClosureLimits,
    ) -> (Result<usize, String>, Option<&'static str>) {
        use crate::n3::{self, NegationCycles};
        let run = Truncation::top_level_with(limits);
        let r = n3::reason_n3_terms_in(doc, None, None, NegationCycles::Reject, &run);
        (r.map(|c| c.facts.len()), run.get())
    }

    /// Rounds the `n`-edge linear chain needs: one per edge, plus the round that finds
    /// nothing new and the closing check of the stratum.
    const fn linear_rounds(n: usize) -> usize {
        n + LINEAR_EXTRA_ROUNDS
    }
    const LINEAR_EXTRA_ROUNDS: usize = 2;

    #[test]
    fn a_long_linear_walk_closes_under_the_default_allowance() {
        // 100,000 edges: about 100,000 rounds and 200,000 facts, a finite workload a
        // fixed 100,000-round cap refused. The default allowance closes it, with no cut.
        let n = 100_000;
        let (r, cut) = close_under(&linear_chain(n), ClosureLimits::default());
        assert_eq!(r, Ok(2 * n + 1));
        assert_eq!(cut, None);
    }

    #[test]
    fn a_caller_sets_the_round_allowance_of_a_linear_walk() {
        use crate::n3::{self, NegationCycles};
        let n = 2_000;
        let doc = linear_chain(n);
        let enough = ClosureLimits {
            max_rounds: linear_rounds(n),
            ..ClosureLimits::default()
        };
        let short = ClosureLimits {
            max_rounds: linear_rounds(n) - 1,
            ..ClosureLimits::default()
        };
        // Just enough rounds: the closure completes, with no cut.
        assert_eq!(close_under(&doc, enough), (Ok(2 * n + 1), None));
        let closed =
            n3::reason_n3_terms_with_limits(&doc, None, None, NegationCycles::Reject, enough)
                .expect("closes within its allowance");
        assert_eq!(closed.facts.len(), 2 * n + 1);
        let ids = n3::reason_n3_with_limits(
            &mut sparq_core::dict::Dict::new(),
            &doc,
            NegationCycles::Reject,
            enough,
        )
        .expect("closes within its allowance");
        assert_eq!(ids.len(), 2 * n + 1);
        // One round fewer: an error and a cut, never the truncated closure.
        let (r, cut) = close_under(&doc, short);
        assert_closure_limit("linear walk, one round short", r);
        assert_eq!(cut, Some(CLOSURE_CUT));
        assert_closure_limit(
            "reason_n3_terms_with_limits",
            n3::reason_n3_terms_with_limits(&doc, None, None, NegationCycles::Reject, short)
                .map(|c| c.facts.len()),
        );
        assert_closure_limit(
            "reason_n3_with_limits",
            n3::reason_n3_with_limits(
                &mut sparq_core::dict::Dict::new(),
                &doc,
                NegationCycles::Reject,
                short,
            ),
        );
        // A walk shorter than the allowance is unaffected by it.
        assert_eq!(
            close_under(&linear_chain(n - 10), short),
            (Ok(2 * (n - 10) + 1), None)
        );
    }

    #[test]
    fn a_nested_closure_inherits_the_runs_limits() {
        use crate::n3::{self, NegationCycles};
        // The walk runs inside a `log:conclusion` closure: the run's limits reach it.
        let mut inner = String::new();
        inner.push_str(":start :reaches :n0 . { :start :reaches ?x. ?x :next ?y } => { :start :reaches ?y } . ");
        for i in 0..50 {
            inner.push_str(&format!(":n{i} :next :n{} . ", i + 1));
        }
        let doc = format!(
            "@prefix : <http://example.org/#>.\n\
             @prefix log: <http://www.w3.org/2000/10/swap/log#>.\n\
             :w :is {{ {inner} }} .\n\
             {{ :w :is ?f. ?f log:conclusion ?c }} => {{ :w :closed ?c }} .\n"
        );
        let roomy = ClosureLimits {
            max_rounds: 1_000,
            ..ClosureLimits::default()
        };
        assert!(
            n3::reason_n3_terms_with_limits(&doc, None, None, NegationCycles::Reject, roomy)
                .is_ok()
        );
        let tight = ClosureLimits {
            max_rounds: 20,
            ..ClosureLimits::default()
        };
        assert_closure_limit(
            "nested closure under a tight allowance",
            n3::reason_n3_terms_with_limits(&doc, None, None, NegationCycles::Reject, tight)
                .map(|c| c.facts.len()),
        );
    }

    /// GH #6771: each round doubles a literal, so the closure holds a few dozen facts
    /// after a few dozen rounds when the term outgrows memory.
    const STRING_DOUBLING: &str = "@prefix : <http://ex/>.\n\
         @prefix string: <http://www.w3.org/2000/10/swap/string#>.\n\
         :a :p \"x\".\n\
         { :a :p ?s. (?s ?s) string:concatenation ?t } => { :a :p ?t }.\n";

    #[test]
    fn a_growing_term_is_refused_with_a_cut() {
        // The default limits: refused, with the cut recorded, long before memory runs out.
        let (r, cut) = close_under(STRING_DOUBLING, ClosureLimits::default());
        assert_closure_limit("string doubling", r);
        assert_eq!(cut, Some(CLOSURE_CUT));
        // The largest term derived is the last doubling at or under the limit: under a
        // limit of 4096 bytes that is a 2048-character string (its successor would take
        // 4096 bytes of text plus the literal's datatype and node).
        let small = ClosureLimits {
            max_term_bytes: 4096,
            ..ClosureLimits::default()
        };
        let run = Truncation::top_level_with(small);
        let e = crate::n3::reason_n3_terms_in(
            STRING_DOUBLING,
            None,
            None,
            crate::n3::NegationCycles::Reject,
            &run,
        )
        .map(|c| c.facts.len())
        .expect_err("refused");
        assert!(e.contains("max_term_bytes (4096)"), "{e}");
        assert_eq!(run.get(), Some(CLOSURE_CUT));
    }

    #[test]
    fn the_term_checkpoint_refuses_only_past_the_limit() {
        let limits = ClosureLimits {
            max_term_bytes: 100,
            ..ClosureLimits::default()
        };
        let run = Truncation::top_level_with(limits);
        let never = || -> usize { panic!("the exact size is computed only past the bound") };
        assert_eq!(cut_of(derived_term(&run, &"f", 100, never)), (false, false));
        // A loose bound over the limit, an exact size under it: allowed.
        assert_eq!(cut_of(derived_term(&run, &"f", 101, || 100)), (false, false));
        assert!(run.refusal().is_none());
        assert_eq!(cut_of(derived_term(&run, &"f", 101, || 101)), (true, true));
        assert!(run.refusal().is_some_and(|e| e.contains(CLOSURE_CUT)));

        let s = Term::Lit("abc".into(), "http://www.w3.org/2001/XMLSchema#string".into(), None);
        assert_eq!(term_bytes(&s), string_term_bytes(3));
        let list = Term::List(vec![s.clone(), Term::List(vec![s.clone()])]);
        assert_eq!(term_bytes(&list), 2 * TERM_NODE_BYTES + 2 * term_bytes(&s));
        let formula = Term::Formula(vec![[s.clone(), s.clone(), s.clone()]]);
        assert_eq!(term_bytes(&formula), TERM_NODE_BYTES + 3 * term_bytes(&s));
        // A deep term is walked without recursion.
        let mut deep = Term::List(vec![]);
        for _ in 0..200_000 {
            deep = Term::List(vec![deep]);
        }
        assert_eq!(term_bytes(&deep), 200_001 * TERM_NODE_BYTES);
        // Dropping it would recurse; leak it instead.
        std::mem::forget(deep);
    }

    #[test]
    fn each_limit_reports_a_cut_and_keeps_its_value() {
        let run = Truncation::top_level();
        let step = |depth, could_match| {
            let (s, cut) = cut_of(backward_step(&run, depth, could_match));
            (s.map(|(d, _)| d), cut)
        };
        assert_eq!(step(3, true), (Some(2), false));
        assert_eq!(step(0, true), (None, true));
        assert_eq!(step(0, false), (None, false));
        // The run's stack allowance: held while a step runs, given back when it ends.
        let held: Vec<_> = (0..RUN_STACK).map(|_| take_stack(&run, 1)).collect();
        assert!(held.iter().all(Option::is_some));
        assert_eq!(step(3, true), (None, true));
        assert_eq!(step(3, false), (None, false));
        assert!(enter_closure(&run).is_err());
        drop(held);
        assert_eq!(step(3, true), (Some(2), false));
        assert!(enter_closure(&run).is_ok());

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
        assert_eq!(e.message(), format!("nesting deeper than {PARSE_DEPTH}"));
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
            .stack_size(2 << 20)
            .spawn(move || {
                let sink = Cell::new(false);
                let doc = parse_n3(&deep, "", &sink);
                (doc.err(), sink.get())
            })
            .expect("spawn")
            .join()
            .expect("join");
        let deep = (
            deep.0 == Some(format!("nesting deeper than {PARSE_DEPTH}")),
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

    /// #6732: `depth` counts every list, formula and quoted triple as a level, an empty
    /// one included, so it agrees with `fits`.
    #[test]
    fn depth_counts_empty_containers_as_fits_does() {
        use super::super::Term;
        let atom = Term::Iri("http://ex/a".into());
        let empties = [
            atom.clone(),
            Term::List(Vec::new()),
            Term::Formula(Vec::new()),
            Term::Triple(Box::new([atom.clone(), atom.clone(), atom])),
        ];
        for innermost in empties {
            let mut t = innermost;
            for wraps in 0..4 {
                let d = depth(&t);
                assert!(fits(&t, d), "{t:?} fits its depth");
                assert!(d == 0 || !fits(&t, d - 1), "{t:?} needs its depth");
                if wraps % 2 == 0 {
                    t = Term::List(vec![t]);
                } else {
                    t = Term::Formula(vec![[
                        Term::Iri("http://ex/s".into()),
                        Term::Iri("http://ex/p".into()),
                        t,
                    ]]);
                }
            }
        }
        assert_eq!(depth(&Term::List(vec![Term::List(Vec::new())])), 2);
        assert_eq!(depth(&Term::Formula(Vec::new())), 1);
    }

    /// WHICH terms the walk borrows: a term it left as it found it is `Borrowed`, and only
    /// a compound it rebuilt member by member is `Owned`. That is the whole content of
    /// `walked_atom_cow`, and an ordinary goal's three terms are leaves, so an ordinary
    /// goal copies nothing.
    ///
    /// This test deliberately does NOT claim to check the two walks against each other on
    /// the terms or the refusals: `walked_atom` IS `walked_atom_cow(..).map(into_owned)`,
    /// so comparing them would pass whatever the one implementation did. What pins the
    /// walk's terms and refusals is the three `walked_atom` tests below, which run through
    /// that same single implementation.
    #[test]
    fn walked_atom_cow_borrows_every_term_it_did_not_rebuild() {
        use std::borrow::Cow;
        let iri = |s: &str| Term::Iri(s.to_string());
        let var = |s: &str| Term::Var(s.to_string());
        let run = Truncation::top_level();
        let mut b = super::super::Binding::default();
        b.insert("bound", iri("http://ex/v"));
        b.insert("chain", var("bound"));
        b.insert("list", Term::List(vec![var("chain"), iri("http://ex/w")]));
        b.insert("cyc", Term::Formula(vec![[var("cyc"), iri("http://ex/q"), var("bound")]]));
        let goals = vec![
            // Every term a leaf: nothing is copied.
            [iri("http://ex/a"), iri("http://ex/p"), var("free")],
            [var("bound"), iri("http://ex/p"), var("chain")],
            // A compound is rebuilt, so it is owned; its siblings are still borrowed.
            [var("list"), iri("http://ex/p"), var("bound")],
            [Term::List(vec![var("chain"), var("chain")]), iri("http://ex/p"), var("free")],
            // The cyclic formula the occurs check stops.
            [var("cyc"), iri("http://ex/p"), var("free")],
            [
                Term::Triple(Box::new([var("bound"), iri("http://ex/p"), var("free")])),
                iri("http://ex/p"),
                iri("http://ex/o"),
            ],
        ];
        // Every goal here is admissible under the default limits; a refused one would
        // carry no terms to classify, and what the walk refuses is pinned by the
        // `walked_atom` tests below.
        for g in &goals {
            let walked = walked_atom_cow(g, &b, &run).expect("admissible under the defaults");
            for (i, ct) in walked.iter().enumerate() {
                // A term the walk did not rebuild is BORROWED; a compound it built member
                // by member is OWNED. The resolved term's own shape decides which, so this
                // reads the shape off the walked term rather than off the input.
                let compound =
                    matches!(ct.as_ref(), Term::List(_) | Term::Triple(_) | Term::Formula(_));
                assert_eq!(
                    matches!(ct, Cow::Owned(_)),
                    compound,
                    "term {i} of {g:?} is owned exactly when it was rebuilt"
                );
            }
        }
    }

    /// #6844: `walked_atom` resolves every variable of a goal to the END of its chain,
    /// inside lists and quoted triples, keeps a quoted formula as the value it is, and
    /// stops a chain that walks back into itself instead of following it forever.
    #[test]
    fn walked_atom_resolves_chains_and_stops_a_cyclic_one() {
        use super::super::{Binding, Term};
        let iri = |s: &str| Term::Iri(format!("http://ex/{}", s));
        let var = |s: &str| Term::Var(s.to_string());
        let mut b = Binding::default();
        b.insert("x", var("mid"));
        b.insert("mid", var("end"));
        b.insert("end", iri("v"));
        b.insert("inner", iri("leaked"));
        let quoted = Term::Formula(vec![
            [iri("s"), iri("p"), var("inner")],
            [iri("s"), iri("q"), var("own")],
        ]);
        let goal = [
            Term::List(vec![var("x"), Term::Triple(Box::new([iri("s"), iri("p"), var("x")]))]),
            iri("has"),
            quoted.clone(),
        ];
        let run = Truncation::top_level();
        let walked = walked_atom(&goal, &b, &run).expect("within the run's allowance");
        assert_eq!(
            walked[0],
            Term::List(vec![iri("v"), Term::Triple(Box::new([iri("s"), iri("p"), iri("v")]))]),
            "the chain is followed to its end in a list member and in a quoted triple"
        );
        // A quoted formula is walked too (GH #6844 round 4): the variable the CALLER bound
        // resolves, because the proof reads the formula through that binding, while the
        // formula's OWN variable — not bound in `b` — is left as it is.
        assert_eq!(
            walked[2],
            Term::Formula(vec![[iri("s"), iri("p"), iri("leaked")], [iri("s"), iri("q"), var("own")]]),
            "the caller's binding inside the formula resolves; the formula's own does not"
        );
        assert_ne!(walked[2], quoted, "which is not the opaque value");
        // Substitution follows ONE binding, which is what made two goals look alike.
        assert_eq!(substitute(&goal[0], &b, &run), Some(apply(&goal[0], &b)));
        assert_ne!(apply(&goal[0], &b), walked[0], "one binding is not the whole chain");

        // A chain that walks back into itself stops at the re-entry (GH #6844 round 5): the
        // occurs check leaves the second `?x` as the variable it is, so the walk is finite
        // and the goal still HAS an identity instead of being refused for its depth.
        let mut cyclic = Binding::default();
        cyclic.insert("x", Term::List(vec![var("x")]));
        let cyclic_run = Truncation::top_level();
        assert_eq!(
            walked_atom(&[var("x"), iri("has"), iri("v")], &cyclic, &cyclic_run),
            Some([Term::List(vec![var("x")]), iri("has"), iri("v")]),
            "a cycle is stopped at its re-entry, not followed and not refused"
        );
        assert_eq!(cyclic_run.get(), None, "and the run is not cut short by it");
    }

    /// #6844 round 4: only a compound takes a level off the walker's room, so a goal
    /// admitted AT the depth boundary — its innermost members are leaves reached with no
    /// room left — is walked, not refused, and does not underflow the counter.
    #[test]
    fn walked_atom_walks_a_goal_at_the_depth_boundary() {
        use super::super::{Binding, Term};
        let run = Truncation::top_level();
        let room = run.term_depth.get();
        let mut t = Term::Iri("http://ex/deep".into());
        for _ in 0..room {
            t = Term::List(vec![t]);
        }
        assert_eq!(depth(&t), room, "nested to exactly the run's allowance");
        assert!(fits(&t, room), "and admitted by the allowance");
        let goal = [t.clone(), Term::Iri("http://ex/p".into()), Term::Iri("http://ex/o".into())];
        let walked =
            walked_atom(&goal, &Binding::default(), &run).expect("a goal at the boundary walks");
        assert_eq!(walked[0], t, "nothing was bound, so nothing changed");
        // One level deeper is past the allowance, and that is a refusal rather than a panic.
        let deeper = [Term::List(vec![t]), goal[1].clone(), goal[2].clone()];
        assert_eq!(walked_atom(&deeper, &Binding::default(), &run), None, "one level too deep");
    }

    /// #6844 round 4: `max_term_bytes` bounds ONE term, so the walker counts each term of a
    /// goal on its own. Three terms that are each admissible and together are not must still
    /// walk.
    #[test]
    fn walked_atom_counts_each_term_of_a_goal_on_its_own() {
        use super::super::{Binding, Term};
        let lit = |n: usize| {
            Term::Lit("x".repeat(n), "http://www.w3.org/2001/XMLSchema#string".into(), None)
        };
        let limits = ClosureLimits { max_term_bytes: 2_000, ..ClosureLimits::default() };
        let run = Truncation::top_level_with(limits);
        let goal = [lit(900), Term::Iri("http://ex/p".into()), lit(900)];
        assert!(
            goal.iter().all(|t| term_bytes(t) <= 2_000),
            "each term is admissible on its own"
        );
        assert!(
            goal.iter().map(term_bytes).sum::<usize>() > 2_000,
            "and their sum is not"
        );
        assert_eq!(
            walked_atom(&goal, &Binding::default(), &run),
            Some(goal.clone()),
            "a per-term limit is not a per-atom one"
        );
        // A SINGLE term past the limit is still refused.
        let big = [lit(3_000), goal[1].clone(), goal[2].clone()];
        assert_eq!(walked_atom(&big, &Binding::default(), &run), None, "one term too large");
    }
}
