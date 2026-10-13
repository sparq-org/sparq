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
use std::rc::Rc;

use super::model::Term;

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
/// run (nested ones included) gets the allowance its entry point was given, and the run's
/// refusal: the error of the first term refused for its size ([`derived_term`]), which
/// fails every closure of the run that sees it.
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

/// Why a forward closure was stopped by its [`ClosureLimits`].
const CLOSURE_CUT: &str = "forward closure limit exceeded";

/// The allowance of one N3 forward closure: how many rounds it may run, how many facts it
/// may hold, and how large a term a functional builtin may derive, before it is refused.
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
/// sized once the parse returns it. Numeric results are not covered (below). The size is
/// approximate: string bytes plus a fixed cost per node.
/// Binding a variable to a term that already exists (premise matching, `log:includes`)
/// copies that term; it builds nothing new and is not sized again.
///
/// Substitution is sized because the raw substitution functions are private to this
/// module; the guard test `substitution_goes_through_the_bounded_module` is a lexical
/// tripwire for a crate-internal copy, not the guarantee.
///
/// What it does not bound:
/// * the closure's total memory: many distinct terms just under the limit are bounded
///   only by `max_facts`;
/// * the parser input size: a parse (the document's own, or a `log:parsedAsN3` /
///   `log:semantics` one) allocates the terms it builds before its result is sized, so
///   its memory is bounded by its input text, and a short prefix used many times
///   expands before the formula is refused;
/// * numeric results, as on main (GH #6901): the forward and reverse results of the
///   arithmetic and other numeric builtins, and the canonical numeric pieces
///   `string:concatenation` renders. Their size entries are fixed estimates that do not
///   follow a decimal's scale, and the rendered form is not checked against this limit
///   anywhere;
/// * the text a resolver supplies (`log:content`, `log:semantics`), which the caller
///   allocates before it is sized (the literal or formula built from it is refused);
/// * the incremental counting engine behind `MaterializedN3Graph` (GH #6854; its batch
///   fallback is bounded) and the opt-in `compiled-rules` evaluator (GH #6855), neither
///   of which has the round and fact allowance either.
///
/// One over-refusal is deliberate: a builtin's argument list is a term it builds, so
/// `log:conjunction` over many copies of one large formula is refused when the copies
/// together pass the limit, even if the merged result would fit.
///
/// The struct is `#[non_exhaustive]`: outside this crate, start from
/// [`ClosureLimits::default`] and set the fields to change
/// (`let mut l = ClosureLimits::default(); l.max_term_bytes = 1 << 20;`).
///
/// Pass one to [`reason_n3_with_limits`](crate::n3::reason_n3_with_limits) or
/// [`reason_n3_terms_with_limits`](crate::n3::reason_n3_terms_with_limits); every other
/// N3 entry point uses [`ClosureLimits::default`].
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
    /// triples. The terms it covers are checked before they are built; numeric results
    /// are not covered (see the type's docs, GH #6901). Default 16 MiB.
    pub max_term_bytes: usize,
}

impl Default for ClosureLimits {
    /// 1,000,000 rounds, 50,000,000 facts and 16 MiB per derived term.
    fn default() -> Self {
        ClosureLimits {
            max_rounds: CLOSURE_ROUNDS,
            max_facts: CLOSURE_FACTS,
            max_term_bytes: CLOSURE_TERM_BYTES,
        }
    }
}

/// The fixed cost [`term_bytes`] charges each node of a term.
pub(crate) const TERM_NODE_BYTES: usize = std::mem::size_of::<Term>();

/// The approximate size of `t` that [`ClosureLimits::max_term_bytes`] bounds: the bytes
/// of its strings plus [`TERM_NODE_BYTES`] per node, through nested lists, formulas and
/// quoted triples (walked with an explicit stack, so a deep term cannot overflow it).
pub(crate) fn term_bytes(t: &Term) -> usize {
    let flat = |t: &Term| match t {
        Term::Iri(s) | Term::Blank(s) | Term::Var(s) => Some(s.len()),
        Term::Lit(v, dt, lang) => Some(v.len() + dt.len() + lang.as_ref().map_or(0, String::len)),
        Term::Formula(_) | Term::List(_) | Term::Triple(_) => None,
    };
    if let Some(n) = flat(t) {
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
            leaf => total = total.saturating_add(flat(leaf).unwrap_or(0)),
        }
    }
    total
}

/// The [`term_bytes`] of an `xsd:string` literal whose lexical form has `lex` bytes.
pub(crate) fn string_term_bytes(lex: usize) -> usize {
    const XSD_STRING_LEN: usize = "http://www.w3.org/2001/XMLSchema#string".len();
    lex.saturating_add(XSD_STRING_LEN + TERM_NODE_BYTES)
}

/// The ONE checkpoint the terms the text engine sizes pass before they are built
/// (GH #6771; numeric results are not covered, GH #6901). `bound` is an upper bound on the
/// term's [`term_bytes`], computed from
/// borrowed inputs without building anything; `exact` is its exact size, computed (again
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

/// The [`term_bytes`] of `t` with `b`'s bindings substituted, as [`substitute`] (or, with
/// `deep`, [`substitute_deep`], through quoted formulae) would build it, read from the
/// borrowed terms without building anything.
pub(crate) fn resolved_bytes(t: &Term, b: &super::Binding, deep: bool) -> usize {
    let all = |ts: &mut dyn Iterator<Item = &Term>| {
        ts.fold(TERM_NODE_BYTES, |n, m| n.saturating_add(resolved_bytes(m, b, deep)))
    };
    match t {
        Term::Var(v) => term_bytes(b.get(v).unwrap_or(t)),
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
/// A compound `t` can name a bound term many times, so it is sized from the borrowed
/// terms first ([`resolved_bytes`]) and refused past the run's term-size limit
/// ([`derived_term`]): `None`, with the cut and the run's refusal recorded. A variable or
/// atom is one copy of an existing term and is not sized. `tests/n3_bounded_discipline.rs`
/// fails if an N3 evaluator substitutes any other way.
#[inline]
pub(crate) fn substitute(t: &Term, b: &super::Binding, run: &Truncation) -> Option<Term> {
    match t {
        Term::List(_) | Term::Triple(_) if compound_refused(t, b, run, false) => None,
        _ => Some(apply(t, b)),
    }
}

/// [`substitute`], also substituting inside quoted formulae (a conclusion's
/// instantiation): a variable left inside a formula after substitution is the formula's
/// own.
#[inline]
pub(crate) fn substitute_deep(t: &Term, b: &super::Binding, run: &Truncation) -> Option<Term> {
    match t {
        Term::List(_) | Term::Triple(_) | Term::Formula(_) if compound_refused(t, b, run, true) => {
            None
        }
        _ => Some(apply_deep(t, b)),
    }
}

/// Whether compound `t`, substituted under `b`, is past the run's term-size limit
/// (sized from the borrowed terms; the cut and the run's refusal are recorded). Kept out
/// of line: a variable or atom, the common case, never reaches it.
#[inline(never)]
fn compound_refused(t: &Term, b: &super::Binding, run: &Truncation, deep: bool) -> bool {
    let bytes = resolved_bytes(t, b, deep);
    settle(run, derived_term(run, &Substituted, bytes, || bytes))
}

/// Substitute bound variables in `t` (recursing into list members and quoted triples);
/// returns the term (possibly still containing free vars). Private: callers go through
/// [`substitute`].
fn apply(t: &Term, b: &super::Binding) -> Term {
    match t {
        Term::Var(v) => b.get(v).cloned().unwrap_or_else(|| t.clone()),
        Term::List(ms) => Term::List(ms.iter().map(|m| apply(m, b)).collect()),
        Term::Triple(tr) => {
            Term::Triple(Box::new([apply(&tr[0], b), apply(&tr[1], b), apply(&tr[2], b)]))
        }
        _ => t.clone(),
    }
}

/// [`apply`] through quoted formulae too. Private: callers go through
/// [`substitute_deep`].
fn apply_deep(t: &Term, b: &super::Binding) -> Term {
    match t {
        Term::Var(v) => match b.get(v) {
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
            _ => {
                cuts.record(CLOSURE_CUT);
                Err(format!(
                    "n3: {CLOSURE_CUT} ({} rounds, {} facts) before reaching a fixpoint: \
                     the rules keep deriving new facts (raise ClosureLimits to allow more)",
                    self.limits.max_rounds, self.limits.max_facts
                ))
            }
        }
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

    /// The GH #6757 programs: a backward rule concludes a formula holding a variable (or
    /// an `@forAll` universal), and a forward rule copies it out. Every backward proof
    /// renames the variable apart, so each round derives a new, syntactically distinct
    /// fact and the closure never saturates.
    const DIVERGING: [&str; 4] = [
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
}
