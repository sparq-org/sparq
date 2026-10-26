//! Reusable work accounting for explicitly metered operations.
//!
//! Limits use `usize::MAX` as the unbounded sentinel. Refused charges leave all
//! counters unchanged. Bytes are aggregate; document bytes reset at each document.
//! This module does not install a global allocator or meter existing loaders.

use crate::dict::{is_inline, split_lang_dir, Dict, Id, TermParts, INLINE_BASE};
use oxrdf::Term;
use std::fmt;

/// Fixed cost per RDF term node, in addition to its lexical bytes.
pub const TERM_NODE_BYTES: usize = std::mem::size_of::<Term>();
/// Maximum triple-term nesting, matching the dictionary's reconstruction guard.
pub const MAX_TRIPLE_DEPTH: usize = 64;

/// A dimension of explicitly charged work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Dim {
    /// Source bytes accepted by an explicitly metered reader.
    SourceBytes,
    /// Aggregate lexical and fixed-node bytes charged across documents.
    Bytes,
    /// Bytes charged in the current document, reset by `Ledger::open_document`.
    DocumentBytes,
    /// Per-term size cap, checked without debiting it by term-sizing operations.
    TermBytes,
    /// Facts accepted by an explicitly metered operation.
    Facts,
    /// Nodes accepted by an explicitly metered operation.
    Nodes,
    /// Current entered depth, balanced by `Ledger::enter` and `Ledger::leave`.
    Depth,
    /// Query result rows charged by a consumer.
    QueryRows,
    /// Premise join rows and rule firings charged across closures.
    JoinRows,
    /// Facts held by successful closures, charged across closures.
    ClosureFacts,
    /// Fixed triple-term nesting guard, independent of `Limits::depth`.
    TermDepth,
    /// Allocation failure, reported only by fallible allocation helpers.
    Alloc,
}

impl Dim {
    /// Stable snake_case name used in error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::SourceBytes => "source_bytes",
            Self::Bytes => "bytes",
            Self::DocumentBytes => "document_bytes",
            Self::TermBytes => "term_bytes",
            Self::Facts => "facts",
            Self::Nodes => "nodes",
            Self::Depth => "depth",
            Self::QueryRows => "query_rows",
            Self::JoinRows => "join_rows",
            Self::ClosureFacts => "closure_facts",
            Self::TermDepth => "term_depth",
            Self::Alloc => "alloc",
        }
    }
}

/// Limits for one ledger; `usize::MAX` means unbounded in every field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Maximum accepted source bytes.
    pub source_bytes: usize,
    /// Maximum aggregate lexical and fixed-node bytes across documents.
    pub bytes: usize,
    /// Maximum bytes in each document; the current value resets on opening one.
    pub document_bytes: usize,
    /// Maximum size of each term; this cap is checked but never debited by term sizing.
    pub term_bytes: usize,
    /// Maximum accepted facts.
    pub facts: usize,
    /// Maximum accepted nodes.
    pub nodes: usize,
    /// Maximum simultaneously entered depth levels; leaving a level refunds it.
    pub depth: usize,
    /// Maximum query result rows.
    pub query_rows: usize,
    /// Maximum aggregate premise join rows and rule firings.
    pub join_rows: usize,
    /// Maximum aggregate facts held by successful closures.
    pub closure_facts: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: usize::MAX,
            bytes: usize::MAX,
            document_bytes: usize::MAX,
            term_bytes: usize::MAX,
            facts: usize::MAX,
            nodes: usize::MAX,
            depth: usize::MAX,
            query_rows: usize::MAX,
            join_rows: usize::MAX,
            closure_facts: usize::MAX,
        }
    }
}

/// A refused charge and the total it would have reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Exceeded {
    /// Dimension whose limit was exceeded, or `Dim::Alloc` for allocation failure.
    pub dim: Dim,
    /// Configured cap, fixed triple-depth guard, or `usize::MAX` for allocation failure.
    pub limit: usize,
    /// Refused total, or requested resulting element count for allocation failure.
    pub reached: usize,
}

impl Exceeded {
    /// Creates a typed refusal for consumers implementing their own capped operations.
    pub const fn new(dim: Dim, limit: usize, reached: usize) -> Self {
        Self {
            dim,
            limit,
            reached,
        }
    }
}

impl fmt::Display for Exceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dim == Dim::Alloc {
            return write!(f, "allocation for {} elements failed", self.reached);
        }
        write!(
            f,
            "Work limit {} ({}) exceeded by a charge reaching {}.",
            self.dim.name(),
            self.limit,
            self.reached
        )
    }
}

impl std::error::Error for Exceeded {}

/// Borrowed lexical components of an RDF term.
///
/// `Triple` borrows an oxrdf triple rather than recursively constructing adapters;
/// [`From<&Term>`] therefore allocates nothing even for deeply nested triple terms.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum TermRef<'a> {
    Iri(&'a str),
    Blank(&'a str),
    Literal {
        value: &'a str,
        datatype: &'a str,
        lang: Option<&'a str>,
    },
    Triple(&'a oxrdf::Triple),
}

impl<'a> From<&'a Term> for TermRef<'a> {
    fn from(t: &'a Term) -> Self {
        match t {
            Term::NamedNode(n) => Self::Iri(n.as_str()),
            Term::BlankNode(b) => Self::Blank(b.as_str()),
            Term::Literal(l) => Self::Literal {
                value: l.value(),
                datatype: l.datatype().as_str(),
                lang: l.language(),
            },
            Term::Triple(t) => Self::Triple(t),
        }
    }
}

/// Running work counters with atomic refusal of compound byte charges.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Ledger {
    limits: Limits,
    spent: [usize; 12],
}

impl Ledger {
    /// Creates a ledger with no work spent.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            spent: [0; 12],
        }
    }

    /// Returns the configured limits.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn limit(&self, dim: Dim) -> usize {
        match dim {
            Dim::SourceBytes => self.limits.source_bytes,
            Dim::Bytes => self.limits.bytes,
            Dim::DocumentBytes => self.limits.document_bytes,
            Dim::TermBytes => self.limits.term_bytes,
            Dim::Facts => self.limits.facts,
            Dim::Nodes => self.limits.nodes,
            Dim::Depth => self.limits.depth,
            Dim::QueryRows => self.limits.query_rows,
            Dim::JoinRows => self.limits.join_rows,
            Dim::ClosureFacts => self.limits.closure_facts,
            Dim::TermDepth => MAX_TRIPLE_DEPTH,
            Dim::Alloc => usize::MAX,
        }
    }

    fn check(&self, dim: Dim, spent: usize, n: usize) -> Result<(), Exceeded> {
        let limit = self.limit(dim);
        let reached = spent.saturating_add(n);
        if limit != usize::MAX && reached > limit {
            return Err(Exceeded {
                dim,
                limit,
                reached,
            });
        }
        Ok(())
    }

    /// Checks and then debits work, using saturating counters.
    ///
    /// `Bytes` also charges `DocumentBytes`; both checks precede either debit.
    /// `Alloc` has no limit and is reserved for allocation diagnostics.
    /// # Errors
    /// Returns the first exceeded limit, without debiting any counter.
    pub fn charge(&mut self, dim: Dim, n: usize) -> Result<(), Exceeded> {
        self.check(dim, self.spent(dim), n)?;
        if dim == Dim::Bytes {
            self.check(Dim::DocumentBytes, self.spent(Dim::DocumentBytes), n)?;
            self.spent[Dim::DocumentBytes as usize] =
                self.spent(Dim::DocumentBytes).saturating_add(n);
        }
        self.spent[dim as usize] = self.spent(dim).saturating_add(n);
        Ok(())
    }

    /// Returns the allowance left, or `usize::MAX` for an unbounded dimension.
    pub fn remaining(&self, dim: Dim) -> usize {
        let limit = self.limit(dim);
        if limit == usize::MAX {
            usize::MAX
        } else {
            limit.saturating_sub(self.spent(dim))
        }
    }

    /// Returns spent work.
    ///
    /// `DocumentBytes` and `Depth` are current values, not cumulative totals.
    /// Term-sizing operations never debit `TermBytes` or `TermDepth`.
    pub fn spent(&self, dim: Dim) -> usize {
        self.spent[dim as usize]
    }

    /// Starts a document, resetting only its byte counter.
    pub fn open_document(&mut self) {
        self.spent[Dim::DocumentBytes as usize] = 0;
    }

    fn sized(&mut self, size: usize) -> Result<usize, Exceeded> {
        self.check(Dim::TermBytes, 0, size)?;
        self.charge(Dim::Bytes, size)?;
        Ok(size)
    }

    /// Sizes a term iteratively and charges its bytes, including current document bytes.
    /// # Errors
    /// Refuses excessive triple depth, one term's size, or the compound byte charge.
    pub fn term(&mut self, t: TermRef<'_>) -> Result<usize, Exceeded> {
        let mut total: usize = 0;
        // RDF 1.2 only nests through the object; a one-slot explicit stack suffices.
        let mut stack = Some((t, 0));
        while let Some((t, depth)) = stack.take() {
            total = total.saturating_add(TERM_NODE_BYTES);
            let lexical = match t {
                TermRef::Iri(s) | TermRef::Blank(s) => s.len(),
                TermRef::Literal {
                    value,
                    datatype,
                    lang,
                } => value
                    .len()
                    .saturating_add(datatype.len())
                    .saturating_add(lang.map_or(0, str::len)),
                TermRef::Triple(t) => {
                    check_depth(depth)?;
                    total = total.saturating_add(2 * TERM_NODE_BYTES);
                    stack = Some((TermRef::from(&t.object), depth + 1));
                    match &t.subject {
                        oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().len(),
                        oxrdf::NamedOrBlankNode::BlankNode(b) => b.as_str().len(),
                    }
                    .saturating_add(t.predicate.as_str().len())
                }
            };
            total = total.saturating_add(lexical);
        }
        self.sized(total)
    }

    /// Sizes dictionary parts without reconstructing terms or inline integer strings.
    /// # Errors
    /// As [`Self::term`]. The triple-depth walk uses a fixed-size explicit stack.
    pub fn stored_term(&mut self, dict: &Dict, id: Id) -> Result<usize, Exceeded> {
        let mut total: usize = 0;
        // At most two siblings remain per nesting level. No heap allocation is needed.
        let mut stack = [(id, 0); 2 * MAX_TRIPLE_DEPTH + 1];
        let mut len = 1;
        while len > 0 {
            len -= 1;
            let (id, depth) = stack[len];
            total = total.saturating_add(TERM_NODE_BYTES);
            if is_inline(id) {
                let n = id - INLINE_BASE;
                let digits = if n == 0 { 1 } else { n.ilog10() as usize + 1 };
                total = total
                    .saturating_add(digits)
                    .saturating_add(oxrdf::vocab::xsd::INTEGER.as_str().len());
                continue;
            }
            let lexical = match dict.term_parts(id) {
                TermParts::Iri { prefix, suffix } => prefix.len().saturating_add(suffix.len()),
                TermParts::Blank(s) => s.len(),
                TermParts::Lit {
                    value,
                    datatype,
                    lang,
                } => value
                    .len()
                    .saturating_add(datatype.len())
                    .saturating_add(lang.map_or(0, |l| split_lang_dir(l).0.len())),
                TermParts::Triple(ids) => {
                    check_depth(depth)?;
                    for child in ids {
                        stack[len] = (child, depth + 1);
                        len += 1;
                    }
                    0
                }
            };
            total = total.saturating_add(lexical);
        }
        self.sized(total)
    }

    /// Charges joined lexical bytes before fallibly allocating exact capacity.
    /// # Errors
    /// Refuses the compound byte charge, or returns `Alloc` and refunds it on reservation failure.
    pub fn mint(&mut self, parts: &[&str]) -> Result<String, Exceeded> {
        self.mint_using(parts, |size| {
            let mut out = String::new();
            out.try_reserve_exact(size)?;
            Ok(out)
        })
    }

    // This seam makes charge-before-allocation ordering and allocation failure
    // observable without unsafe code or a global allocator.
    fn mint_using(
        &mut self,
        parts: &[&str],
        allocate: impl FnOnce(usize) -> Result<String, std::collections::TryReserveError>,
    ) -> Result<String, Exceeded> {
        let size = parts.iter().fold(0usize, |n, s| n.saturating_add(s.len()));
        let before = self.spent;
        self.charge(Dim::Bytes, size)?;
        let mut out = allocate(size).map_err(|_| {
            self.spent = before;
            Exceeded::new(Dim::Alloc, usize::MAX, size)
        })?;
        for part in parts {
            out.push_str(part);
        }
        Ok(out)
    }

    /// Charges elements before fallibly reserving vector capacity.
    /// # Errors
    /// Refuses the charge, or returns `Alloc` and refunds it if reservation fails.
    pub fn reserve<T>(
        &mut self,
        v: &mut Vec<T>,
        additional: usize,
        dim: Dim,
    ) -> Result<(), Exceeded> {
        let before = self.spent;
        self.charge(dim, additional)?;
        if v.try_reserve(additional).is_err() {
            self.spent = before;
            return Err(Exceeded {
                dim: Dim::Alloc,
                limit: usize::MAX,
                reached: v.len().saturating_add(additional),
            });
        }
        Ok(())
    }

    /// Enters one depth level.
    /// # Errors
    /// Refuses a depth past the configured limit without changing the counter.
    pub fn enter(&mut self) -> Result<(), Exceeded> {
        self.charge(Dim::Depth, 1)
    }

    /// Leaves one depth level, saturating at zero.
    pub fn leave(&mut self) {
        self.spent[Dim::Depth as usize] = self.spent(Dim::Depth).saturating_sub(1);
    }
}

fn check_depth(depth: usize) -> Result<(), Exceeded> {
    if depth >= MAX_TRIPLE_DEPTH {
        return Err(Exceeded {
            dim: Dim::TermDepth,
            limit: MAX_TRIPLE_DEPTH,
            reached: depth.saturating_add(1),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode, Triple};

    #[test]
    fn charge_refuses_atomically_and_remaining_is_exact() {
        let mut l = Ledger::new(Limits {
            facts: 3,
            ..Limits::default()
        });
        l.charge(Dim::Facts, 2).unwrap();
        assert_eq!(l.remaining(Dim::Facts), 1);
        let e = l.charge(Dim::Facts, 2).unwrap_err();
        assert_eq!(e, Exceeded::new(Dim::Facts, 3, 4));
        assert_eq!(l.spent(Dim::Facts), 2);
        l.charge(Dim::Facts, 1).unwrap();
        assert_eq!(l.remaining(Dim::Facts), 0);
        assert!(l.charge(Dim::Facts, 1).is_err());
    }

    #[test]
    fn unbounded_counts_and_saturates_without_refusal() {
        let mut l = Ledger::new(Limits::default());
        l.charge(Dim::JoinRows, 7).unwrap();
        assert_eq!(l.spent(Dim::JoinRows), 7);
        assert_eq!(l.remaining(Dim::JoinRows), usize::MAX);
        l.charge(Dim::JoinRows, usize::MAX).unwrap();
        l.charge(Dim::JoinRows, 1).unwrap();
        assert_eq!(l.spent(Dim::JoinRows), usize::MAX);
    }

    #[test]
    fn bytes_charge_document_and_reset_only_document() {
        let mut l = Ledger::new(Limits {
            bytes: 10,
            document_bytes: 6,
            ..Limits::default()
        });
        l.charge(Dim::Bytes, 5).unwrap();
        assert_eq!(l.remaining(Dim::DocumentBytes), 1);
        assert_eq!(l.charge(Dim::Bytes, 2).unwrap_err().dim, Dim::DocumentBytes);
        assert_eq!(l.spent(Dim::Bytes), 5);
        assert_eq!(l.spent(Dim::DocumentBytes), 5);
        l.open_document();
        assert_eq!(l.spent(Dim::DocumentBytes), 0);
        assert_eq!(l.spent(Dim::Bytes), 5);
        assert_eq!(l.charge(Dim::Bytes, 6).unwrap_err().dim, Dim::Bytes);
        assert_eq!(l.spent(Dim::DocumentBytes), 0);
        l.charge(Dim::Bytes, 5).unwrap();
        assert_eq!(l.spent(Dim::DocumentBytes), 5);
        assert_eq!(l.spent(Dim::Bytes), 10);
    }

    fn nested(mut t: Term, n: usize) -> Term {
        for _ in 0..n {
            t = Triple::new(
                BlankNode::new_unchecked("s"),
                NamedNode::new_unchecked("urn:p"),
                t,
            )
            .into();
        }
        t
    }

    #[test]
    fn term_and_stored_term_match_hand_computed_sizes() {
        let plain: Term = Literal::new_simple_literal("abc").into();
        let integer: Term = Literal::new_typed_literal("12", oxrdf::vocab::xsd::INTEGER).into();
        let table: Vec<(Term, usize)> = vec![
            (
                NamedNode::new_unchecked("urn:i").into(),
                TERM_NODE_BYTES + 5,
            ),
            (
                BlankNode::new_unchecked("blank").into(),
                TERM_NODE_BYTES + 5,
            ),
            (
                plain.clone(),
                TERM_NODE_BYTES + 3 + oxrdf::vocab::xsd::STRING.as_str().len(),
            ),
            (
                integer.clone(),
                TERM_NODE_BYTES + 2 + oxrdf::vocab::xsd::INTEGER.as_str().len(),
            ),
            (
                Literal::new_typed_literal("012", oxrdf::vocab::xsd::INTEGER).into(),
                TERM_NODE_BYTES + 3 + oxrdf::vocab::xsd::INTEGER.as_str().len(),
            ),
            (
                Literal::new_language_tagged_literal_unchecked("hi", "en").into(),
                TERM_NODE_BYTES + 4 + oxrdf::vocab::rdf::LANG_STRING.as_str().len(),
            ),
            (
                Literal::new_directional_language_tagged_literal_unchecked(
                    "hi",
                    "en",
                    oxrdf::BaseDirection::Ltr,
                )
                .into(),
                TERM_NODE_BYTES + 4 + oxrdf::vocab::rdf::DIR_LANG_STRING.as_str().len(),
            ),
            (
                Literal::new_directional_language_tagged_literal_unchecked(
                    "hi",
                    "en",
                    oxrdf::BaseDirection::Rtl,
                )
                .into(),
                TERM_NODE_BYTES + 4 + oxrdf::vocab::rdf::DIR_LANG_STRING.as_str().len(),
            ),
            (
                nested(plain, 3),
                10 * TERM_NODE_BYTES + 3 * 6 + 3 + oxrdf::vocab::xsd::STRING.as_str().len(),
            ),
            (
                nested(integer, 3),
                10 * TERM_NODE_BYTES + 3 * 6 + 2 + oxrdf::vocab::xsd::INTEGER.as_str().len(),
            ),
        ];
        let mut dict = Dict::new();
        for (t, size) in table {
            let id = dict.intern(&t);
            let mut l = Ledger::new(Limits::default());
            let term_size = l.term(TermRef::from(&t)).unwrap();
            let stored_size = l.stored_term(&dict, id).unwrap();
            assert_eq!(term_size, size, "{t}");
            assert_eq!(term_size, stored_size, "{t}");
            assert_eq!(l.spent(Dim::TermBytes), 0);
            assert_eq!(l.spent(Dim::Bytes), 2 * size);
            assert_eq!(l.spent(Dim::DocumentBytes), 2 * size);
            let mut refused = Ledger::new(Limits {
                term_bytes: size - 1,
                ..Limits::default()
            });
            assert_eq!(
                refused.term(TermRef::from(&t)).unwrap_err().dim,
                Dim::TermBytes
            );
            assert_eq!(
                refused.stored_term(&dict, id).unwrap_err().dim,
                Dim::TermBytes
            );
            assert_eq!(refused.spent(Dim::Bytes), 0);
        }
        for value in ["0", "9", "10", "999", "1073741823"] {
            let t: Term = Literal::new_typed_literal(value, oxrdf::vocab::xsd::INTEGER).into();
            let id = dict.intern(&t);
            let mut l = Ledger::new(Limits::default());
            assert_eq!(
                l.term((&t).into()).unwrap(),
                l.stored_term(&dict, id).unwrap()
            );
        }
    }

    #[test]
    fn deep_triple_refuses_depth_without_debit() {
        let t = nested(
            Literal::new_simple_literal("x").into(),
            MAX_TRIPLE_DEPTH + 1,
        );
        let mut l = Ledger::new(Limits::default());
        assert_eq!(
            l.term((&t).into()).unwrap_err(),
            Exceeded::new(Dim::TermDepth, MAX_TRIPLE_DEPTH, MAX_TRIPLE_DEPTH + 1)
        );
        let mut dict = Dict::new();
        // Intern one shallow triple at a time, so the dictionary interner never
        // recurses through the deeply nested input. Its public parts seam accepts ids.
        let s = dict.intern(&BlankNode::new_unchecked("s").into());
        let p = dict.intern(&NamedNode::new_unchecked("urn:p").into());
        let mut id = dict.intern(&Literal::new_simple_literal("x").into());
        for _ in 0..=MAX_TRIPLE_DEPTH {
            id = dict.intern_triple_ids([s, p, id]);
        }
        assert_eq!(l.stored_term(&dict, id).unwrap_err().dim, Dim::TermDepth);
        assert_eq!(l.spent(Dim::Bytes), 0);
        let t = nested(Literal::new_simple_literal("x").into(), MAX_TRIPLE_DEPTH);
        assert!(l.term((&t).into()).is_ok());
    }

    #[test]
    fn reserve_refunds_failed_allocation_and_compound_charge() {
        let mut l = Ledger::new(Limits::default());
        l.charge(Dim::Bytes, 3).unwrap();
        let mut v = Vec::<u64>::new();
        let e = l.reserve(&mut v, usize::MAX / 2, Dim::Bytes).unwrap_err();
        assert_eq!(e.dim, Dim::Alloc);
        assert_eq!(l.spent(Dim::Bytes), 3);
        assert_eq!(l.spent(Dim::DocumentBytes), 3);
        assert_eq!(v.capacity(), 0);
        l.reserve(&mut v, 2, Dim::Nodes).unwrap();
        assert_eq!(l.spent(Dim::Nodes), 2);
        assert!(v.capacity() >= 2);
        let mut l = Ledger::new(Limits {
            nodes: 0,
            ..Limits::default()
        });
        let mut v = Vec::<u64>::new();
        assert_eq!(
            l.reserve(&mut v, 1, Dim::Nodes).unwrap_err().dim,
            Dim::Nodes
        );
        assert_eq!(v.capacity(), 0);
    }

    #[test]
    fn mint_refunds_failed_allocation() {
        let mut l = Ledger::new(Limits::default());
        l.charge(Dim::Bytes, 3).unwrap();
        // Real exhaustion cannot be forced safely from valid string slices.
        // Inject a deterministic capacity-overflow reservation error at the allocation seam.
        let e = l
            .mint_using(&["ab", "cd"], |size| {
                assert_eq!(size, 4);
                let mut out = String::new();
                out.try_reserve_exact(usize::MAX)?;
                Ok(out)
            })
            .unwrap_err();
        assert_eq!(e, Exceeded::new(Dim::Alloc, usize::MAX, 4));
        assert_eq!(l.spent(Dim::Bytes), 3);
        assert_eq!(l.spent(Dim::DocumentBytes), 3);
        assert_eq!(e.to_string(), "allocation for 4 elements failed");
        assert_eq!(Dim::TermDepth.name(), "term_depth");
    }

    #[test]
    fn mint_charges_before_allocating_and_depth_balances() {
        let mut l = Ledger::new(Limits {
            bytes: 5,
            depth: 1,
            ..Limits::default()
        });
        assert_eq!(l.mint(&["ab", "cd"]).unwrap(), "abcd");
        assert_eq!(l.spent(Dim::Bytes), 4);
        assert_eq!(l.spent(Dim::DocumentBytes), 4);
        assert_eq!(l.mint(&["ef"]).unwrap_err().dim, Dim::Bytes);
        assert_eq!(
            l.mint_using(&["ef"], |_| panic!("allocation before byte refusal"))
                .unwrap_err()
                .dim,
            Dim::Bytes
        );
        assert_eq!(l.spent(Dim::Bytes), 4);
        l.enter().unwrap();
        assert_eq!(l.enter().unwrap_err().dim, Dim::Depth);
        assert_eq!(l.spent(Dim::Depth), 1);
        l.leave();
        l.leave();
        assert_eq!(l.spent(Dim::Depth), 0);
        l.enter().unwrap();
    }
}
