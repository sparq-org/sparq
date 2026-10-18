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
//!   the number of facts.
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
    /// The same bytes summed over EVERY document the input is assembled from.
    pub max_input_bytes: usize,
    /// Facts the assembled reasoning input may carry, document content and the
    /// structural/provenance/credential facts together.
    pub max_input_facts: usize,
    /// Approximate bytes of ONE term of the input — the counterpart of
    /// [`sparq_reason::ClosureLimits::max_term_bytes`], which bounds only derived terms.
    pub max_input_term_bytes: usize,
    /// Binding rows and rule firings summed over ALL closures of one authorization
    /// materialization (`ClosureLimits::max_total_join_rows` bounds one closure, and a
    /// materialization runs several).
    pub max_decision_join_rows: usize,
    /// Facts summed over the fixpoints of all closures of one authorization
    /// materialization (`ClosureLimits::max_facts` bounds one closure).
    pub max_decision_facts: usize,
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
        max_decision_join_rows: AUTH_MAX_DECISION_JOIN_ROWS,
        max_decision_facts: AUTH_MAX_DECISION_FACTS,
    }
}

/// Bytes one control or group document may contribute to the reasoning input. Smallest
/// value the fixtures still materialize under: 3,568; this is ~18x it.
const AUTH_MAX_DOCUMENT_BYTES: usize = 64 << 10;
/// Bytes all documents together may contribute. Fixture minimum 61,722; ~17x it.
const AUTH_MAX_INPUT_BYTES: usize = 1 << 20;
/// Facts the assembled input may carry. Fixture minimum 2,520; ~13x it.
const AUTH_MAX_INPUT_FACTS: usize = 32_000;
/// Bytes of one input term. Fixture minimum 114; ~36x it — a legitimate ACL can carry a
/// long IRI or a long literal, and this dimension costs nothing to leave roomy.
const AUTH_MAX_INPUT_TERM_BYTES: usize = 4 << 10;
/// Binding rows and firings over ALL closures of one materialization. Fixture minimum
/// 357,200; ~11x it. Deliberately the same value as one closure's `max_total_join_rows`,
/// so an ACP view's three strata TOGETHER may not do more work than a single-stratum WAC
/// view may — the aggregate is never looser than the per-closure cap.
const AUTH_MAX_DECISION_JOIN_ROWS: usize = 4_000_000;
/// Facts over all closures' fixpoints of one materialization. Fixture minimum 34,603;
/// ~15x it, and 2x one closure's `max_facts`, so the strata of an ACP view may not each
/// hold a full closure's worth.
const AUTH_MAX_DECISION_FACTS: usize = 512_000;

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
}

impl InputMeter {
    pub(crate) fn new(budget: AuthBudget) -> InputMeter {
        InputMeter {
            budget,
            bytes: 0,
            doc_bytes: 0,
        }
    }

    /// Begin reading the document `iri`: its per-document allowance starts afresh, while
    /// the aggregate carries over.
    pub(crate) fn open(&mut self) {
        self.doc_bytes = 0;
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

    /// Add one closure's charge, refusing when the decision's aggregate is past either
    /// cap. A closure whose `max_total_join_rows` was left unbounded reports no join rows
    /// (there is nothing to count then), so only its facts are charged.
    pub(crate) fn charge(&mut self, c: ClosureCharge) -> Result<(), String> {
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
        self.join_rows = self.join_rows.saturating_add(c.join_rows.unwrap_or(0));
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
