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
use sparq_core::dict::{Dict, Id};
use sparq_core::meter::{Dim, Exceeded, Ledger, Limits};

/// The input and whole-decision budget one authorization materialization runs under,
/// alongside the per-closure reasoning allowance ([`crate::default_auth_limits`]).
///
/// [`default_auth_budget`] is what a [`crate::PodStore`] applies until it is given
/// another one ([`crate::PodStore::set_auth_budget`]).
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
    /// Generated lexical bytes admitted before allocation, independently of source and
    /// resource-enumeration bytes. Default: 64 GiB on 64-bit targets; unbounded on
    /// narrower targets. Lower it to constrain generated input separately.
    pub max_generated_bytes: usize,
    /// Facts in the assembled input, including accepted source rows and generated facts.
    /// Rejected documents and filtered group rows do not consume this allowance.
    pub max_input_facts: usize,
    /// Approximate bytes of ONE stored input term — the counterpart of
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
    /// Wall-clock allowance for one materialization, including input assembly, every
    /// stratum and replacement-view assembly. Default: five seconds; `None` disables it.
    /// Checked at coarse boundaries and once per closure round, never per rule row.
    /// Rollback and post-commit rebuilds omit this deadline while retaining count/size caps.
    /// Ignored on wasm32, where no monotonic clock is available.
    pub max_materialize_time: Option<std::time::Duration>,
}

impl AuthBudget {
    /// Copy this budget without its elapsed-time limit, preserving all count and size caps.
    /// Rollback and post-commit rebuilds use this budget; request admission keeps the
    /// configured deadline. The original budget is unchanged.
    pub fn without_deadline(&self) -> Self {
        Self {
            max_materialize_time: None,
            ..*self
        }
    }
}

/// The input and whole-decision budget every authorization materialization runs under
/// unless the store is given another one ([`crate::PodStore::set_auth_budget`]).
///
/// Existing count and size defaults are unchanged. Generated lexical strings have
/// a separate allowance; source rows and generated strings are admitted before
/// allocation, and each closure runs under
/// the remaining whole-decision allowance. Fixture parity is checked against the
/// baseline views, decisions and closure charges.
///
/// The native clock is shared by input assembly and all strata, sampled at coarse
/// boundaries and once per closure round. `None` disables it; wasm32 ignores it.
pub fn default_auth_budget() -> AuthBudget {
    AuthBudget {
        max_document_bytes: AUTH_MAX_DOCUMENT_BYTES,
        max_input_bytes: AUTH_MAX_INPUT_BYTES,
        max_generated_bytes: AUTH_MAX_GENERATED_BYTES,
        max_input_facts: AUTH_MAX_INPUT_FACTS,
        max_input_term_bytes: AUTH_MAX_INPUT_TERM_BYTES,
        max_resource_iri_bytes: AUTH_MAX_RESOURCE_IRI_BYTES,
        max_resource_path_depth: AUTH_MAX_RESOURCE_PATH_DEPTH,
        max_decision_join_rows: AUTH_MAX_DECISION_JOIN_ROWS,
        max_decision_facts: AUTH_MAX_DECISION_FACTS,
        max_materialize_time: Some(std::time::Duration::from_secs(5)),
    }
}

/// Approximate stored-term bytes contributed by one source document.
const AUTH_MAX_DOCUMENT_BYTES: usize = 64 << 10;
/// Aggregate source and resource-enumeration bytes.
const AUTH_MAX_INPUT_BYTES: usize = 1 << 20;
/// Generated lexical bytes; narrower targets cannot represent this finite limit.
const AUTH_MAX_GENERATED_BYTES: usize = if usize::BITS >= 64 {
    (64u64 << 30) as usize
} else {
    usize::MAX
};
/// Accepted source rows and generated input facts admitted.
const AUTH_MAX_INPUT_FACTS: usize = 32_000;
/// Approximate bytes of one stored input term.
const AUTH_MAX_INPUT_TERM_BYTES: usize = 4 << 10;
/// Bytes of one resource IRI admitted before walking its containers.
const AUTH_MAX_RESOURCE_IRI_BYTES: usize = 2 << 10;
/// Containers on one resource IRI's chain, checked before walking it.
const AUTH_MAX_RESOURCE_PATH_DEPTH: usize = 64;
/// Binding rows and rule firings across all closures, bounded before each runs.
const AUTH_MAX_DECISION_JOIN_ROWS: usize = 4_000_000;
/// Facts summed across successful closure fixpoints.
const AUTH_MAX_DECISION_FACTS: usize = 512_000;

/// One clock shared by input assembly and all strata of a materialization.
#[derive(Clone, Copy)]
pub(crate) struct MaterializeDeadline {
    #[cfg(not(target_arch = "wasm32"))]
    started: Option<std::time::Instant>,
    #[cfg(not(target_arch = "wasm32"))]
    limit: Option<std::time::Duration>,
}

impl MaterializeDeadline {
    pub(crate) fn new(_budget: AuthBudget) -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            started: _budget
                .max_materialize_time
                .map(|_| std::time::Instant::now()),
            #[cfg(not(target_arch = "wasm32"))]
            limit: _budget.max_materialize_time,
        }
    }

    pub(crate) fn instant(&self) -> Option<std::time::Instant> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started
                .zip(self.limit)
                .and_then(|(start, limit)| start.checked_add(limit))
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Coarse cooperative check; wasm32 has no monotonic clock and does no timing.
    pub(crate) fn check(&self) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        if self
            .limit
            .zip(self.started)
            .is_some_and(|(limit, started)| started.elapsed() >= limit)
        {
            return Err(allowance_refusal(
                AuthAllowance::Deadline,
                "max_materialize_time elapsed; no authorization view is installed (fail-closed)",
            ));
        }
        Ok(())
    }
}

/// One ledger for input assembly and every closure of a materialization.
pub(crate) struct AuthLedger {
    budget: AuthBudget,
    deadline: MaterializeDeadline,
    core: Ledger,
}

impl AuthLedger {
    pub(crate) fn new(budget: AuthBudget) -> Self {
        Self::with_deadline(budget, MaterializeDeadline::new(budget))
    }

    pub(crate) fn with_deadline(budget: AuthBudget, deadline: MaterializeDeadline) -> Self {
        let mut limits = Limits::default();
        limits.document_bytes = budget.max_document_bytes;
        limits.bytes = budget.max_input_bytes;
        limits.generated_bytes = budget.max_generated_bytes;
        limits.term_bytes = budget.max_input_term_bytes;
        limits.facts = budget.max_input_facts;
        limits.join_rows = budget.max_decision_join_rows;
        limits.closure_facts = budget.max_decision_facts;
        Self {
            budget,
            deadline,
            core: Ledger::new(limits),
        }
    }

    pub(crate) fn core(&mut self) -> &mut Ledger {
        &mut self.core
    }

    pub(crate) fn deadline(&self) -> Option<std::time::Instant> {
        self.deadline.instant()
    }

    pub(crate) fn checkpoint(&self) -> Result<(), String> {
        self.deadline.check()
    }

    pub(crate) fn open(&mut self) -> Result<(), String> {
        self.checkpoint()?;
        self.core.open_document();
        Ok(())
    }

    /// Stored parts are checked before dictionary reconstruction allocates them.
    pub(crate) fn stored_term(&mut self, iri: &str, dict: &Dict, id: Id) -> Result<(), String> {
        self.core.stored_term(dict, id).map(drop).map_err(|mut e| {
            // The document refusal historically takes precedence over the aggregate.
            if e.dim == Dim::Bytes {
                let n = e.reached.saturating_sub(self.core.spent(Dim::Bytes));
                if n > self.core.remaining(Dim::DocumentBytes) {
                    e = Exceeded::new(
                        Dim::DocumentBytes,
                        self.budget.max_document_bytes,
                        self.core.spent(Dim::DocumentBytes).saturating_add(n),
                    );
                }
            }
            self.refusal(e, Dim::TermBytes, iri)
        })
    }

    /// Reject an impossible source byte contribution before reserving its rows.
    /// Every stored triple contributes at least three fixed term-node costs. Facts
    /// are charged only when accepted rows enter the assembled input.
    pub(crate) fn document_rows<T>(
        &mut self,
        out: &mut Vec<T>,
        n: usize,
        iri: &str,
    ) -> Result<(), String> {
        let row_bytes = 3 * sparq_core::meter::TERM_NODE_BYTES;
        for (dim, limit) in [
            (Dim::DocumentBytes, self.budget.max_document_bytes),
            (Dim::Bytes, self.budget.max_input_bytes),
        ] {
            if limit != usize::MAX && n > self.core.remaining(dim) / row_bytes {
                return Err(self.refusal(
                    Exceeded::new(
                        dim,
                        limit,
                        self.core
                            .spent(dim)
                            .saturating_add(n.saturating_mul(row_bytes)),
                    ),
                    dim,
                    iri,
                ));
            }
        }
        self.reserve(out, n, Dim::Nodes)
    }

    /// Fallible input-sized vector growth, charged before reserving memory.
    pub(crate) fn reserve<T>(
        &mut self,
        out: &mut Vec<T>,
        n: usize,
        dim: Dim,
    ) -> Result<(), String> {
        self.core
            .reserve(out, n, dim)
            .map_err(|e| self.refusal(e, dim, "generated input"))
    }

    /// Size one generated IRI before allocating any of its lexical parts.
    pub(crate) fn mint(&mut self, parts: &[&str]) -> Result<String, String> {
        // Generated strings have their own dimension; existing source, document and
        // stored-term caps retain their scope.
        self.core.close_document();
        self.core
            .mint_generated(parts)
            .map_err(|e| self.refusal(e, Dim::GeneratedBytes, "generated input"))
    }

    pub(crate) fn resource_name(&mut self, iri: &str) -> Result<(), String> {
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

    /// A resource is admitted by length, depth, then its aggregate byte charge.
    pub(crate) fn resource(&mut self, iri: &str) -> Result<(), String> {
        self.resource_name(iri)?;
        self.resource_bytes(iri.len())
    }

    pub(crate) fn resource_bytes(&mut self, n: usize) -> Result<(), String> {
        self.core.close_document();
        self.core.charge(Dim::Bytes, n).map_err(|_| {
            allowance_refusal(
                AuthAllowance::InputBytes,
                &format!(
                    "this pod's resources and their containers contribute more than \
                 max_input_bytes ({}) to the reasoning input; the authorization view is \
                 not built (fail-closed)",
                    self.budget.max_input_bytes
                ),
            )
        })
    }

    pub(crate) fn copied_term(&mut self, term: &oxrdf::Term) -> Result<oxrdf::Term, String> {
        match term {
            oxrdf::Term::NamedNode(n) => self
                .mint(&[n.as_str()])
                .map(|s| oxrdf::NamedNode::new_unchecked(s).into()),
            oxrdf::Term::Literal(literal) => {
                let bytes = literal
                    .value()
                    .len()
                    .saturating_add(literal.datatype().as_str().len())
                    .saturating_add(literal.language().map_or(0, str::len));
                self.core.close_document();
                self.core
                    .charge(Dim::GeneratedBytes, bytes)
                    .map_err(|e| self.refusal(e, Dim::GeneratedBytes, "generated input"))?;
                Ok(term.clone())
            }
            _ => {
                let mut current = Some(term);
                let mut bytes = 0usize;
                while let Some(term) = current.take() {
                    let lexical = match term {
                        oxrdf::Term::NamedNode(n) => n.as_str().len(),
                        oxrdf::Term::BlankNode(b) => b.as_str().len(),
                        oxrdf::Term::Literal(l) => l
                            .value()
                            .len()
                            .saturating_add(l.datatype().as_str().len())
                            .saturating_add(l.language().map_or(0, str::len)),
                        oxrdf::Term::Triple(t) => {
                            current = Some(&t.object);
                            let subject = match &t.subject {
                                oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().len(),
                                oxrdf::NamedOrBlankNode::BlankNode(b) => b.as_str().len(),
                            };
                            subject.saturating_add(t.predicate.as_str().len())
                        }
                    };
                    bytes = bytes.saturating_add(lexical);
                }
                self.core
                    .charge(Dim::GeneratedBytes, bytes)
                    .map_err(|e| self.refusal(e, Dim::GeneratedBytes, "generated input"))?;
                Ok(term.clone())
            }
        }
    }

    /// Percent-encode into a single fallibly reserved string, after its full charge.
    pub(crate) fn skolem(
        &mut self,
        prefix: &str,
        graph: &str,
        label: &str,
    ) -> Result<String, String> {
        let unreserved =
            |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        let extra = graph
            .bytes()
            .chain(label.bytes())
            .filter(|b| !unreserved(*b))
            .count()
            .saturating_mul(2);
        let size = prefix
            .len()
            .saturating_add("graph:".len())
            .saturating_add(graph.len())
            .saturating_add(1)
            .saturating_add(label.len())
            .saturating_add(extra);
        self.core.close_document();
        self.core
            .charge(Dim::GeneratedBytes, extra)
            .map_err(|e| self.refusal(e, Dim::GeneratedBytes, graph))?;
        let mut out = self.mint(&[prefix, "graph:", graph, ":", label])?;
        out.try_reserve_exact(extra).map_err(|_| {
            self.refusal(
                Exceeded::new(Dim::Alloc, usize::MAX, size),
                Dim::GeneratedBytes,
                graph,
            )
        })?;
        out.clear();
        out.push_str(prefix);
        out.push_str("graph:");
        for (i, part) in [graph, label].iter().enumerate() {
            if i == 1 {
                out.push(':');
            }
            for b in part.bytes() {
                if unreserved(b) {
                    out.push(char::from(b));
                } else {
                    const HEX: &[u8] = b"0123456789ABCDEF";
                    out.push('%');
                    out.push(char::from(HEX[usize::from(b >> 4)]));
                    out.push(char::from(HEX[usize::from(b & 15)]));
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn refusal(&self, e: Exceeded, reserving: Dim, iri: &str) -> String {
        let dim = if e.dim == Dim::Alloc {
            reserving
        } else {
            e.dim
        };
        let (kind, detail) = match dim {
            Dim::TermDepth => (
                AuthAllowance::InputTermBytes,
                format!(
                    "a term of the access-control document <{}> exceeds the stored triple term \
                     nesting guard ({}); the authorization view is not built (fail-closed)",
                    iri, e.limit
                ),
            ),
            Dim::TermBytes => (
                AuthAllowance::InputTermBytes,
                format!(
                    "a term of the access-control document <{}> is about {} bytes, past the \
                 max_input_term_bytes budget ({}); the authorization view is not built \
                 (fail-closed)",
                    iri, e.reached, self.budget.max_input_term_bytes
                ),
            ),
            Dim::DocumentBytes => (
                AuthAllowance::InputBytes,
                format!(
                    "the access-control document <{}> contributes more than \
                 max_document_bytes ({}) to the reasoning input; the authorization view \
                 is not built (fail-closed)",
                    iri, self.budget.max_document_bytes
                ),
            ),
            Dim::Bytes | Dim::SourceBytes => (
                AuthAllowance::InputBytes,
                format!(
                    "the access-control documents together contribute more than \
                 max_input_bytes ({}) to the reasoning input (reached while reading <{}>); \
                 the authorization view is not built (fail-closed)",
                    self.budget.max_input_bytes, iri
                ),
            ),
            Dim::GeneratedBytes => (
                AuthAllowance::GeneratedBytes,
                format!(
                    "generated lexical strings contribute more than max_generated_bytes ({}); \
                     the authorization view is not built (fail-closed)",
                    self.budget.max_generated_bytes
                ),
            ),
            Dim::JoinRows => (
                AuthAllowance::DecisionWork,
                format!(
                    "the closures of this authorization materialization have produced {} \
                 binding rows and rule firings between them, past the \
                 max_decision_join_rows budget ({}); no view is installed (fail-closed)",
                    e.reached, self.budget.max_decision_join_rows
                ),
            ),
            Dim::ClosureFacts => (
                AuthAllowance::DecisionWork,
                format!(
                    "the closures of this authorization materialization have held {} facts \
                 between them, past the max_decision_facts budget ({}); no view is \
                 installed (fail-closed)",
                    e.reached, self.budget.max_decision_facts
                ),
            ),
            Dim::Deadline => (
                AuthAllowance::Deadline,
                "max_materialize_time elapsed; no authorization view is installed (fail-closed)"
                    .into(),
            ),
            _ => (
                AuthAllowance::InputFacts,
                format!(
                    "the assembled reasoning input is {} facts, past the max_input_facts budget \
                 ({}); the authorization view is not built (fail-closed)",
                    e.reached, self.budget.max_input_facts
                ),
            ),
        };
        let detail = if e.dim == Dim::Alloc {
            format!("{e}; no authorization view is installed (fail-closed)")
        } else {
            detail
        };
        allowance_refusal(kind, &detail)
    }
}

/// Temporarily omit the request clock while rebuilding committed or restored content.
/// Drop restores it even when the rebuild unwinds.
pub(crate) fn untimed_rebuild<T>(
    store: &mut crate::PodStore,
    rebuild: impl FnOnce(&mut crate::PodStore) -> T,
) -> T {
    struct Restore<'a> {
        store: &'a mut crate::PodStore,
        budget: AuthBudget,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            self.store.auth_budget = self.budget;
        }
    }
    let budget = store.auth_budget;
    store.auth_budget = budget.without_deadline();
    let scope = Restore { store, budget };
    rebuild(scope.store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{Literal, NamedNode, Term, Triple};

    #[test]
    fn stored_depth_refusal_reports_nesting_without_debiting_bytes() {
        let mut dict = Dict::new();
        let mut term: Term = Literal::new_simple_literal("v").into();
        for _ in 0..=sparq_core::meter::MAX_TRIPLE_DEPTH {
            term = Triple::new(
                NamedNode::new_unchecked("urn:s"),
                NamedNode::new_unchecked("urn:p"),
                term,
            )
            .into();
        }
        let object = dict.intern(&term);
        // Main already bounds reconstruction at this depth. Even its bounded
        // placeholder reconstruction exceeds the unchanged default term-byte cap.
        let reconstructed = dict.term(object);
        let mut fixed_nodes = 1usize;
        let mut cursor = &reconstructed;
        while let Term::Triple(triple) = cursor {
            fixed_nodes += 3;
            cursor = &triple.object;
        }
        assert!(
            fixed_nodes * sparq_core::meter::TERM_NODE_BYTES
                > default_auth_budget().max_input_term_bytes
        );
        let mut ledger = AuthLedger::new(default_auth_budget());
        let error = ledger
            .stored_term("https://pod.ex/.acl", &dict, object)
            .unwrap_err();
        assert_eq!(
            crate::auth_allowance_refusal(&error),
            Some(AuthAllowance::InputTermBytes)
        );
        assert!(error.contains("nesting guard"), "{error}");
        assert!(
            !error.contains("is about"),
            "depth is not a byte count: {error}"
        );
        assert_eq!(ledger.core().spent(Dim::Bytes), 0);
    }
}
