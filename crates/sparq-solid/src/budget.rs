//! The INPUT and whole-decision budget of an authorization materialization.
//!
//! The reasoning allowance ([`crate::default_auth_limits`]) bounds what the RULES may do
//! inside ONE closure. Two things sit outside it:
//!
//! * **The input.** The control documents a pod holds become owned triples and are
//!   interned into the closure's dictionary before any rule runs
//!   ([`crate::loader`]), and the reasoner's `max_term_bytes` bounds only the terms its
//!   builtins DERIVE — not a term an untrusted writer put in a document. So the input
//!   itself needs a budget: per-document and aggregate bytes, the length of one term, and
//!   the number of facts. The input also carries the pod's RESOURCE ENUMERATION, which
//!   owns a prefix string per container on every resource's chain and so grows with the
//!   resource IRIs rather than with the documents' bytes or the fact count — bounded here
//!   by a length and a depth cap per IRI and by the aggregate byte budget over all of them.
//! * **The whole decision.** [`sparq_reason::ClosureLimits`] applies per CLOSURE, and one
//!   authorization decision runs several: ACP runs three strata (accepts → rejections →
//!   grants) and the ODRL bridge runs five, each its own closure getting the allowance
//!   afresh. The aggregate across them needs its own cap.
//!
//! Both fail closed with the same typed [`crate::AuthAllowance`] refusal family the
//! reasoning allowance uses, and both are configurable through
//! [`crate::PodStore::set_auth_budget`].

use crate::materialize::{allowance_refusal, AuthAllowance};
use oxrdf::Term;
use sparq_reason::n3::compiled::ClosureCharge;
use sparq_reason::ClosureLimits;

/// The input and whole-decision budget one authorization materialization runs under,
/// alongside the per-closure reasoning allowance ([`crate::default_auth_limits`]).
///
/// [`default_auth_budget`] is what a [`crate::PodStore`] applies until it is given
/// another one ([`crate::PodStore::set_auth_budget`]); that function's docs record how
/// each value was measured.
///
/// The struct is `#[non_exhaustive]`: start from [`default_auth_budget`] and set the
/// fields to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AuthBudget {
    /// Approximate bytes ONE control or group document may contribute to the reasoning
    /// input: its terms' lexical bytes plus a fixed cost per term.
    pub max_document_bytes: usize,
    /// The same bytes summed over EVERY document the input is assembled from, plus the
    /// prefix strings of the resource enumeration (see
    /// [`AuthBudget::max_resource_iri_bytes`]).
    pub max_input_bytes: usize,
    /// Facts the assembled reasoning input may carry, document content and the
    /// structural/provenance/credential facts together.
    pub max_input_facts: usize,
    /// Approximate bytes of ONE term of the input — the counterpart of
    /// [`sparq_reason::ClosureLimits::max_term_bytes`], which bounds only derived terms.
    pub max_input_term_bytes: usize,
    /// Bytes of ONE resource IRI the input's resource enumeration may walk. A resource of
    /// the view brings every container on its chain, so an IRI of length L with D
    /// containers makes D owned prefix strings totalling up to L x D bytes — a cost that
    /// grows with the IRI itself, is paid before any fact exists to count, and that
    /// neither [`AuthBudget::max_document_bytes`] nor [`AuthBudget::max_input_facts`]
    /// bounds. This caps the L of that product, [`AuthBudget::max_resource_path_depth`]
    /// caps the D, and the strings are charged against
    /// [`AuthBudget::max_input_bytes`] as they are made.
    pub max_resource_iri_bytes: usize,
    /// Containers one resource IRI's chain may have — the D of the product
    /// [`AuthBudget::max_resource_iri_bytes`] caps the L of. Charged before the chain is
    /// walked, so a deeper IRI is refused instead of walked.
    pub max_resource_path_depth: usize,
    /// Binding rows and rule firings summed over ALL closures of one authorization
    /// materialization (`ClosureLimits::max_total_join_rows` bounds one closure, and a
    /// materialization runs several).
    pub max_decision_join_rows: usize,
    /// Facts summed over the fixpoints of all closures of one authorization
    /// materialization (`ClosureLimits::max_facts` bounds one closure).
    pub max_decision_facts: usize,
    /// WALL-CLOCK time one whole authorization materialization may take: its input
    /// assembly, its resource enumeration and every closure of every stratum together.
    ///
    /// Every other field here bounds a COUNT, which is what makes a refusal reproducible —
    /// but counts do not bound the TIME they take, and a server answering a request has a
    /// wall-clock budget whatever the counts say. This is that bound. It is checked
    /// COARSELY and deliberately so: once per document and per resource while the input is
    /// assembled, once at each stratum boundary, and once per closure round inside the
    /// reasoner (`sparq_reason::ClosureLimits::deadline`) — never per fact or per row, so
    /// the check costs one `Instant::now` per round rather than one per unit of work. It is
    /// therefore an upper bound on when a materialization STOPS, not a promise about any
    /// instant inside a round.
    ///
    /// Because it depends on how fast the machine is and what else is running on it, a
    /// `Deadline` refusal is the one refusal in this family that is not a property of the
    /// pod alone. The count limits are what refuse a pod reproducibly; this is the backstop
    /// that keeps a request from hanging.
    pub max_materialize_time: std::time::Duration,
}

/// The input and whole-decision budget every authorization materialization runs under
/// unless the store is given another one ([`crate::PodStore::set_auth_budget`]).
///
/// # How the values were chosen
///
/// Measured, not guessed, the same way [`crate::default_auth_limits`] was: over the
/// largest fixtures this crate has — `crate::wac_fixture`, the same fixture with filler
/// content (`crate::fixture::wac_fixture_sized(50)`) and `crate::acp_fixture` across all
/// its strata — the SMALLEST value each field could take while the view still
/// materialized was found by bisection, and each value here is about ten times the largest
/// of those, rounded up. Those minima are recorded in the per-constant docs below rather
/// than in any markdown, since they are specific to these fixtures and this box.
///
/// The two resource-enumeration fields are sized the same way against the same fixtures,
/// but from their MAXIMA rather than by bisection: a resource IRI's length and its
/// container depth are properties of the pod's names, so what the fixtures need is the
/// longest and the deepest name they hold, not a value a run either fits in or does not.
///
/// (A cross-check on the whole-decision figure: WAC runs ONE closure, and its measured
/// `max_decision_join_rows` minimum is exactly the `max_total_join_rows` minimum
/// [`crate::default_auth_limits`] records for the same fixture — the aggregate ledger and
/// the per-closure allowance count the same rows.)
///
/// The margins are deliberate but not open-ended: an untrusted writer of a control
/// document chooses how large and how multiplying its policies are, so these ceilings are
/// what they can spend. A pod that genuinely needs more raises them.
pub fn default_auth_budget() -> AuthBudget {
    AuthBudget {
        max_document_bytes: AUTH_MAX_DOCUMENT_BYTES,
        max_input_bytes: AUTH_MAX_INPUT_BYTES,
        max_input_facts: AUTH_MAX_INPUT_FACTS,
        max_input_term_bytes: AUTH_MAX_INPUT_TERM_BYTES,
        max_resource_iri_bytes: AUTH_MAX_RESOURCE_IRI_BYTES,
        max_resource_path_depth: AUTH_MAX_RESOURCE_PATH_DEPTH,
        max_decision_join_rows: AUTH_MAX_DECISION_JOIN_ROWS,
        max_decision_facts: AUTH_MAX_DECISION_FACTS,
        max_materialize_time: AUTH_MAX_MATERIALIZE_TIME,
    }
}

/// Bytes one control or group document may contribute to the reasoning input. Smallest
/// value the fixtures still materialize under: 3,568; this is ~18x it.
const AUTH_MAX_DOCUMENT_BYTES: usize = 64 << 10;
/// Bytes all documents AND the resource enumeration together may contribute. Fixture
/// minimum 98,054; ~10x it.
const AUTH_MAX_INPUT_BYTES: usize = 1 << 20;
/// Facts the assembled input may carry. Fixture minimum 2,520; ~13x it.
const AUTH_MAX_INPUT_FACTS: usize = 32_000;
/// Bytes of one input term. Fixture minimum 114; ~36x it — a legitimate ACL can carry a
/// long IRI or a long literal, and this dimension costs nothing to leave roomy.
const AUTH_MAX_INPUT_TERM_BYTES: usize = 4 << 10;
/// Bytes of one resource IRI the enumeration may walk. Longest resource IRI over the
/// fixtures: 36 bytes; this is ~56x it — the dimension a legitimate pod is likeliest to
/// want room in (a long percent-encoded name), and the enumeration's real bound is the
/// aggregate [`AUTH_MAX_INPUT_BYTES`] rather than this.
const AUTH_MAX_RESOURCE_IRI_BYTES: usize = 2 << 10;
/// Containers one resource IRI's chain may have. Deepest over the fixtures: 4; this is 16x
/// it. The product with [`AUTH_MAX_RESOURCE_IRI_BYTES`] is what one resource's chain may
/// own (128 KiB), and the aggregate caps the sum of those at [`AUTH_MAX_INPUT_BYTES`].
const AUTH_MAX_RESOURCE_PATH_DEPTH: usize = 64;
/// Binding rows and firings over ALL closures of one materialization. Fixture minimum
/// 357,200; ~11x it. Deliberately the same value as one closure's `max_total_join_rows`,
/// so the ACCEPTED aggregate of an ACP view's three strata is no looser than what a
/// single-stratum WAC view may spend.
///
/// This bounds a REFUSED decision as well as an accepted one. The ledger is charged when a
/// closure RETURNS ([`DecisionWork::charge`]), so on its own it would let the closure that
/// crosses the aggregate run to completion under its own per-closure cap first — a peak of
/// this value plus one closure's `max_total_join_rows`. Each closure is therefore handed the
/// decision's REMAINING allowance as its own cap ([`DecisionWork::remaining`]), so the round
/// that would cross this is the round that stops.
const AUTH_MAX_DECISION_JOIN_ROWS: usize = 4_000_000;
/// Facts over all closures' fixpoints of one materialization. Fixture minimum 34,603;
/// ~15x it, and 2x one closure's `max_facts`, so the strata of an ACP view may not each
/// hold a full closure's worth. As with [`AUTH_MAX_DECISION_JOIN_ROWS`], a closure is given
/// what the decision has LEFT as its own `max_facts`, so this bounds a refused decision too
/// rather than only an accepted one.
const AUTH_MAX_DECISION_FACTS: usize = 512_000;

/// Wall-clock time one whole materialization may take. Orders of magnitude above what this
/// crate's own fixtures take to materialize on the boxes it was measured on — a backstop
/// against a materialization that hangs, not a budget a legitimate pod is expected to
/// approach. It is deliberately NOT sized the way the count limits were (by bisecting the
/// smallest value the fixtures still pass under): a time is not a property of the pod, so a
/// tight multiple of a fixture's time on one box would refuse the same pod on a slower one.
/// An embedder that knows its own hardware and request deadline should set its own
/// ([`crate::PodStore::set_auth_budget`]).
const AUTH_MAX_MATERIALIZE_TIME: std::time::Duration = std::time::Duration::from_secs(5);

/// The fixed cost [`term_bytes`] charges per term node, on top of its lexical bytes — the
/// same shape the reasoner's own term sizing uses.
const TERM_NODE_BYTES: usize = std::mem::size_of::<Term>();

/// The approximate bytes of `t` the input budget charges: the UTF-8 bytes of its lexical
/// components (IRI, blank-node label, or a literal's value plus datatype plus language
/// tag) plus [`TERM_NODE_BYTES`] per node, walked through quoted triples with an explicit
/// stack so a deeply-nested term cannot overflow one.
pub(crate) fn term_bytes(t: &Term) -> usize {
    let mut total: usize = 0;
    let mut stack: Vec<&Term> = vec![t];
    while let Some(t) = stack.pop() {
        total = total.saturating_add(TERM_NODE_BYTES);
        match t {
            Term::NamedNode(n) => total = total.saturating_add(n.as_str().len()),
            Term::BlankNode(b) => total = total.saturating_add(b.as_str().len()),
            Term::Literal(l) => {
                total = total.saturating_add(l.value().len());
                total = total.saturating_add(l.datatype().as_str().len());
                total = total.saturating_add(l.language().map_or(0, str::len));
            }
            // A quoted triple: its subject and predicate are node types with no `Term`
            // to borrow, so they are sized directly; only the object can nest further.
            Term::Triple(tr) => {
                total = total.saturating_add(2 * TERM_NODE_BYTES);
                total = total.saturating_add(match &tr.subject {
                    oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().len(),
                    oxrdf::NamedOrBlankNode::BlankNode(b) => b.as_str().len(),
                });
                total = total.saturating_add(tr.predicate.as_str().len());
                stack.push(&tr.object);
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
    }
    total
}

/// The WALL-CLOCK deadline of ONE authorization materialization
/// ([`AuthBudget::max_materialize_time`]), shared by every stage of it so the bound is on
/// the whole run rather than on each stage separately.
///
/// Cheap by construction: it holds the instant it must stop at, so a check is one
/// `Instant::now` and one comparison, and the call sites are coarse (per document, per
/// resource, per stratum, per closure round) rather than per fact or per row.
///
/// On `wasm32-unknown-unknown` there is no monotonic clock (`Instant::now` panics there),
/// so a deadline is never taken and never checked on that target: the count limits are the
/// whole bound there, as they were before this existed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deadline {
    /// The instant the materialization must stop by, or `None` for no deadline at all —
    /// an unrepresentable one (a budget so large it overflows the clock) and every
    /// deadline on wasm32.
    at: Option<std::time::Instant>,
}

impl Deadline {
    /// The deadline of a materialization starting NOW under `budget`.
    pub(crate) fn starting_now(budget: AuthBudget) -> Deadline {
        #[cfg(not(target_arch = "wasm32"))]
        let at = std::time::Instant::now().checked_add(budget.max_materialize_time);
        #[cfg(target_arch = "wasm32")]
        let at = {
            let _ = budget;
            None
        };
        Deadline { at }
    }

    /// No deadline: nothing is sampled and nothing is ever refused. Used by the in-crate
    /// differential tests, whose fixtures are compared fact-for-fact and must not depend on
    /// how fast the machine running them is.
    #[cfg(test)]
    pub(crate) fn none() -> Deadline {
        Deadline { at: None }
    }

    /// The instant to hand the reasoner ([`sparq_reason::ClosureLimits::deadline`]) so its
    /// own round boundaries check the SAME deadline.
    pub(crate) fn at(&self) -> Option<std::time::Instant> {
        self.at
    }

    /// Refuse when the deadline has passed. `stage` names where the materialization was
    /// when it noticed — a `&'static str`, so the refusal carries no pod or request data.
    pub(crate) fn check(&self, stage: &'static str) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.at.is_some_and(|at| std::time::Instant::now() >= at) {
            return Err(allowance_refusal(
                AuthAllowance::Deadline,
                &format!(
                    "this authorization materialization was still {} at its \
                     max_materialize_time deadline; the authorization view is not built \
                     (fail-closed)",
                    stage
                ),
            ));
        }
        let _ = stage;
        Ok(())
    }
}

/// The input budget of one authorization materialization, charged while the input is
/// READ: each term is sized as it is materialized, so a document set past the budget stops
/// being read at the limit instead of being held whole and checked afterwards.
///
/// Every refusal carries the typed [`AuthAllowance`] kind, so a caller tells "this pod's
/// control documents are too big for the budget in force" from any other materialization
/// failure exactly as it does for the reasoning allowance.
pub(crate) struct InputMeter {
    budget: AuthBudget,
    /// Bytes charged over all documents so far.
    bytes: usize,
    /// Bytes charged for the document being read.
    doc_bytes: usize,
    /// The materialization's wall-clock deadline, checked at the COARSE boundaries of the
    /// assembly loops — once per document and once per resource, never per term.
    deadline: Deadline,
}

impl InputMeter {
    /// A meter over `budget` with a deadline of its own, starting now: what a pass that is
    /// not part of one materialization's input assembly takes (the store-side resource
    /// enumerations), so such a pass is bounded in time as well as in bytes.
    pub(crate) fn new(budget: AuthBudget) -> InputMeter {
        InputMeter::with_deadline(budget, Deadline::starting_now(budget))
    }

    /// A meter over `budget` sharing `deadline` — what one materialization's input
    /// assembly takes, so its assembly and its closures run against ONE deadline for the
    /// whole run rather than one each.
    pub(crate) fn with_deadline(budget: AuthBudget, deadline: Deadline) -> InputMeter {
        InputMeter {
            budget,
            bytes: 0,
            doc_bytes: 0,
            deadline,
        }
    }

    /// Begin reading the document `iri`: its per-document allowance starts afresh, while
    /// the aggregate carries over. The deadline is checked here — once per document, which
    /// is the assembly loop's own boundary.
    pub(crate) fn open(&mut self) -> Result<(), String> {
        self.doc_bytes = 0;
        self.deadline.check("reading the access-control documents")
    }

    /// Charge one term of the document being read: its length against
    /// [`AuthBudget::max_input_term_bytes`], and its bytes against the per-document and
    /// aggregate byte budgets.
    pub(crate) fn term(&mut self, iri: &str, t: &Term) -> Result<(), String> {
        let n = term_bytes(t);
        if n > self.budget.max_input_term_bytes {
            return Err(allowance_refusal(
                AuthAllowance::InputTermBytes,
                &format!(
                    "a term of the access-control document <{}> is about {} bytes, past the \
                     max_input_term_bytes budget ({}); the authorization view is not built \
                     (fail-closed)",
                    iri, n, self.budget.max_input_term_bytes
                ),
            ));
        }
        self.doc_bytes = self.doc_bytes.saturating_add(n);
        if self.doc_bytes > self.budget.max_document_bytes {
            return Err(allowance_refusal(
                AuthAllowance::InputBytes,
                &format!(
                    "the access-control document <{}> contributes more than \
                     max_document_bytes ({}) to the reasoning input; the authorization view \
                     is not built (fail-closed)",
                    iri, self.budget.max_document_bytes
                ),
            ));
        }
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > self.budget.max_input_bytes {
            return Err(allowance_refusal(
                AuthAllowance::InputBytes,
                &format!(
                    "the access-control documents together contribute more than \
                     max_input_bytes ({}) to the reasoning input (reached while reading <{}>); \
                     the authorization view is not built (fail-closed)",
                    self.budget.max_input_bytes, iri
                ),
            ));
        }
        Ok(())
    }

    /// Admit `iri` into the resource enumeration: its length against
    /// [`AuthBudget::max_resource_iri_bytes`] and its container depth against
    /// [`AuthBudget::max_resource_path_depth`], BEFORE anything that grows with that depth
    /// is allocated.
    ///
    /// The length is checked first, so the depth walk that follows it is itself bounded
    /// (it stops one step past the depth cap, over an IRI already known to be short).
    pub(crate) fn resource(&mut self, iri: &str) -> Result<(), String> {
        // Once per resource name — the enumeration loop's own boundary.
        self.deadline.check("enumerating this pod's resources")?;
        if iri.len() > self.budget.max_resource_iri_bytes {
            return Err(allowance_refusal(
                AuthAllowance::ResourceIri,
                &format!(
                    "a resource of this pod is named by an IRI of {} bytes, past the \
                     max_resource_iri_bytes budget ({}); the authorization view is not \
                     built (fail-closed)",
                    iri.len(),
                    self.budget.max_resource_iri_bytes
                ),
            ));
        }
        let mut depth = 0usize;
        let mut cur = iri;
        while let Some(parent) = crate::loader::parent_iri(cur) {
            depth += 1;
            if depth > self.budget.max_resource_path_depth {
                return Err(allowance_refusal(
                    AuthAllowance::ResourceIri,
                    &format!(
                        "a resource of this pod is under more than max_resource_path_depth \
                         ({}) containers; the authorization view is not built (fail-closed)",
                        self.budget.max_resource_path_depth
                    ),
                ));
            }
            cur = parent;
        }
        Ok(())
    }

    /// Charge `n` bytes of the resource enumeration — one resource or container string —
    /// against the AGGREGATE [`AuthBudget::max_input_bytes`], before that string is owned.
    ///
    /// Per-resource caps bound one chain; this is what bounds the sum of them, so the
    /// enumeration of a whole pod cannot cost more than the input budget whatever shape
    /// its names have.
    pub(crate) fn resource_bytes(&mut self, n: usize) -> Result<(), String> {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > self.budget.max_input_bytes {
            return Err(allowance_refusal(
                AuthAllowance::InputBytes,
                &format!(
                    "this pod's resources and their containers contribute more than \
                     max_input_bytes ({}) to the reasoning input; the authorization view is \
                     not built (fail-closed)",
                    self.budget.max_input_bytes
                ),
            ));
        }
        Ok(())
    }

    /// Check the ASSEMBLED input — the document content plus the structural, provenance
    /// and credential facts — against [`AuthBudget::max_input_facts`], before it is
    /// interned into a closure's dictionary.
    pub(crate) fn facts(&self, facts: usize) -> Result<(), String> {
        if facts <= self.budget.max_input_facts {
            return Ok(());
        }
        Err(allowance_refusal(
            AuthAllowance::InputFacts,
            &format!(
                "the assembled reasoning input is {} facts, past the max_input_facts budget \
                 ({}); the authorization view is not built (fail-closed)",
                facts, self.budget.max_input_facts
            ),
        ))
    }
}

/// The whole-decision work ledger of one authorization materialization: the charges of
/// every closure it runs, added up and capped
/// ([`AuthBudget::max_decision_join_rows`], [`AuthBudget::max_decision_facts`]).
///
/// [`sparq_reason::ClosureLimits`] gives each closure its rounds, its per-round join-row
/// allowance and its `max_total_join_rows` AFRESH, so a materialization that runs n
/// closures may otherwise spend n times each of them — three for ACP's strata, five for
/// the ODRL bridge's. This is the one number that bounds a whole decision.
pub(crate) struct DecisionWork {
    budget: AuthBudget,
    join_rows: usize,
    facts: usize,
}

impl DecisionWork {
    pub(crate) fn new(budget: AuthBudget) -> DecisionWork {
        DecisionWork {
            budget,
            join_rows: 0,
            facts: 0,
        }
    }

    /// `limits` NARROWED to what this decision has left: each per-closure cap becomes the
    /// smaller of what the caller configured and the decision's remaining allowance.
    ///
    /// Without this a closure is given its per-closure cap AFRESH and the ledger is only
    /// charged when it RETURNS, so ONE stratum can spend a whole decision's allowance before
    /// anything stops it — the ledger then refuses a decision whose peak work has already
    /// been done. Narrowed, the LAST round that fits is the one that stops: the peak of a
    /// refused decision is the decision's own cap rather than that cap plus one closure's.
    ///
    /// The per-ROUND join-row allowance is narrowed too: a round is refilled every round, so
    /// an un-narrowed one could spend the decision's remainder in a single round.
    ///
    /// A cap narrowed to zero refuses the closure at its first round, which is the right
    /// answer — there is nothing left to spend.
    pub(crate) fn remaining(&self, limits: ClosureLimits) -> ClosureLimits {
        let facts_left = self.budget.max_decision_facts.saturating_sub(self.facts);
        let rows_left = self
            .budget
            .max_decision_join_rows
            .saturating_sub(self.join_rows);
        let mut limits = limits;
        limits.max_facts = limits.max_facts.min(facts_left);
        limits.max_total_join_rows = limits.max_total_join_rows.min(rows_left);
        limits.max_join_rows = limits.max_join_rows.min(rows_left);
        limits
    }

    /// Add one closure's charge, refusing when the decision's aggregate is past either cap.
    ///
    /// A closure that reports NO join-row count ran without a `max_total_join_rows` bound,
    /// so what it spent was never counted: charging it zero would let an unbounded closure
    /// pass the ledger untouched. It is refused instead. [`DecisionWork::remaining`] gives
    /// every closure of a decision a bound below the decision's own cap, so a closure that
    /// went through it always reports a count.
    pub(crate) fn charge(&mut self, c: ClosureCharge) -> Result<(), String> {
        let Some(rows) = c.join_rows else {
            return Err(allowance_refusal(
                AuthAllowance::DecisionWork,
                "a closure of this authorization materialization ran without a join-row                  bound, so what it spent cannot be charged against the                  max_decision_join_rows budget; no view is installed (fail-closed)",
            ));
        };
        self.facts = self.facts.saturating_add(c.facts);
        if self.facts > self.budget.max_decision_facts {
            return Err(allowance_refusal(
                AuthAllowance::DecisionWork,
                &format!(
                    "the closures of this authorization materialization have held {} facts \
                     between them, past the max_decision_facts budget ({}); no view is \
                     installed (fail-closed)",
                    self.facts, self.budget.max_decision_facts
                ),
            ));
        }
        self.join_rows = self.join_rows.saturating_add(rows);
        if self.join_rows > self.budget.max_decision_join_rows {
            return Err(allowance_refusal(
                AuthAllowance::DecisionWork,
                &format!(
                    "the closures of this authorization materialization have produced {} \
                     binding rows and rule firings between them, past the \
                     max_decision_join_rows budget ({}); no view is installed (fail-closed)",
                    self.join_rows, self.budget.max_decision_join_rows
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_reason::n3::compiled::{compile, eval_charged, intern_facts};

    /// One real closure's charge, under `limits` — the charges the ledger is fed in
    /// production, rather than a hand-built one.
    fn a_closure_charge(limits: ClosureLimits) -> ClosureCharge {
        let rules = compile("@prefix : <http://ex/> . { ?x a :Man } => { ?x a :Mortal } .")
            .expect("the rule compiles");
        let mut dict = sparq_core::dict::Dict::new();
        let facts = intern_facts(&mut dict, "@prefix : <http://ex/> . :Socrates a :Man .")
            .expect("the fact interns");
        let (_, charge) =
            eval_charged(&mut dict, &facts, &rules, limits).expect("the closure runs");
        charge
    }

    /// A bounded per-closure allowance, so a closure run under it reports what it spent.
    fn bounded() -> ClosureLimits {
        let mut limits = ClosureLimits::default();
        limits.max_total_join_rows = 1_000;
        limits.max_join_rows = 1_000;
        limits
    }

    /// A fresh ledger narrows nothing it was not asked to: a configured cap TIGHTER than the
    /// decision's own stays as it is.
    #[test]
    fn remaining_keeps_a_configured_cap_tighter_than_the_decision() {
        let work = DecisionWork::new(default_auth_budget());
        let mut configured = ClosureLimits::default();
        configured.max_facts = 10;
        configured.max_join_rows = 20;
        configured.max_total_join_rows = 30;

        let narrowed = work.remaining(configured);
        assert_eq!(narrowed.max_facts, 10);
        assert_eq!(narrowed.max_join_rows, 20);
        assert_eq!(narrowed.max_total_join_rows, 30);
    }

    /// What a closure already charged comes OFF the next closure's caps, so no later closure
    /// is handed an allowance the decision cannot afford.
    #[test]
    fn remaining_subtracts_what_the_decision_already_spent() {
        let budget = default_auth_budget();
        let mut work = DecisionWork::new(budget);
        let charge = a_closure_charge(bounded());
        let rows = charge.join_rows.expect("a bounded closure counts its rows");
        let facts = charge.facts;
        assert!(rows > 0 && facts > 0, "the closure did some work");
        work.charge(charge)
            .expect("well inside the decision budget");

        // The reasoner's own unbounded defaults, so the narrowing sets every cap.
        let narrowed = work.remaining(ClosureLimits::default());
        assert_eq!(narrowed.max_facts, budget.max_decision_facts - facts);
        assert_eq!(
            narrowed.max_total_join_rows,
            budget.max_decision_join_rows - rows
        );
        assert_eq!(
            narrowed.max_join_rows,
            budget.max_decision_join_rows - rows,
            "a per-round allowance is refilled every round, so it is narrowed too"
        );
    }

    /// A decision with nothing left narrows to ZERO rather than to its configured cap: the
    /// next closure is refused at its first round instead of spending an allowance that is
    /// already gone.
    #[test]
    fn remaining_narrows_to_zero_when_the_decision_is_spent() {
        let charge = a_closure_charge(bounded());
        let mut budget = default_auth_budget();
        budget.max_decision_facts = charge.facts;
        budget.max_decision_join_rows = charge.join_rows.expect("counted");
        let mut work = DecisionWork::new(budget);
        work.charge(charge)
            .expect("exactly the budget is not past it");

        let narrowed = work.remaining(ClosureLimits::default());
        assert_eq!(narrowed.max_facts, 0);
        assert_eq!(narrowed.max_total_join_rows, 0);
        assert_eq!(narrowed.max_join_rows, 0);
    }

    /// A closure that reports NO join-row count ran without a bound on them, so what it spent
    /// was never counted: it is REFUSED rather than charged zero, which would let an
    /// unbounded closure pass the ledger untouched.
    #[test]
    fn an_unmeasured_closure_is_refused_rather_than_charged_zero() {
        let charge = a_closure_charge(ClosureLimits::default());
        assert!(
            charge.join_rows.is_none(),
            "the reasoner's default leaves the join rows unbounded and uncounted"
        );
        let mut work = DecisionWork::new(default_auth_budget());

        let err = work
            .charge(charge)
            .expect_err("an unmeasured closure must not be charged zero");
        assert_eq!(
            crate::auth_allowance_refusal(&err),
            Some(AuthAllowance::DecisionWork),
            "the refusal is the whole-decision one: {}",
            err
        );
    }
}
