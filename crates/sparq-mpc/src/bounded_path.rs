// sq-py8h.1 — bounded property-path over DISCLOSED global-IRI keys.
//! Bounded-length SPARQL property paths over **disclosed global-IRI keys**
//! (the crypto-free, "headline federation" regime).
//!
//! ## What this is — and the regime boundary (read first)
//!
//! This module resolves **increment #1** of the bounded property-path design
//! (`research/mpc-bounded-property-path-design.md` §6 step 1, §2.1 DISCLOSED
//! regime). It evaluates a **bounded** path `?a (p){m,k} ?b` whose endpoints AND
//! intermediate join keys are **disclosed global IRIs** by *unrolling* the
//! bounded path — statically, on the PUBLIC bound `k` — into a finite set of
//! fixed-length BGP chains, evaluating each chain as a left-to-right fold of the
//! existing [`crate::join::DisclosedKeyJoin`], then UNION-ing and DEDUP-ing the
//! endpoint pairs.
//!
//! Because the EDGE join key on every hop is a disclosed global IRI, the whole
//! computation runs **OUTSIDE the cryptographic core** (architecture convention
//! #4; design §2.1 DISCLOSED regime): it is a plaintext fold of equi-joins, the
//! *exact* construction the `differential_three_holder_chain_equals_union` test
//! in [`crate::join`] already exercises for a single 3-pattern chain — here
//! generalised to a *bounded path* (a union of chains, one per length in
//! `m..=k`). **NO MPC primitive runs; NO secret sharing; NO MPC round occurs.**
//! It is crypto-free, and the differential tests assert that explicitly.
//!
//! ### Regime boundary (stated, not hidden) — disclosed-key ONLY
//!
//! This is the **DISCLOSED-KEY** regime: every key the unroller joins EDGES on is
//! a disclosed global IRI, so nothing enters an MPC. That is a statement about the
//! edge relations ([`DisclosedEdges::from_holder_edges`] enforces it): a chain
//! hop's intermediate `?z_i` is an edge endpoint, hence an IRI. It is NOT a claim
//! about the terms that can REACH a result endpoint or a Sequence midpoint — a
//! zero-length arm contributes `(x, x)` for every term of the DECLARED node
//! domain, which may be a literal or a blank node, and those rows then compose
//! through a Sequence like any other (see
//! [`DisclosedEdges::with_node_domain`]).
//! It does **NOT** support the HIDDEN-intermediate regime (where the `?z_i` are
//! secret and the per-hop match must stay secret-shared via `secure_equal` /
//! `degree_reduce` / `oblivious_set_output`). That is the separate, cryptographic
//! deliverable **sq-py8h.2** (design §2.1 HIDDEN regime, §6 step 2), which THIS
//! bead blocks. Do not read disclosed-key bounded-path support as hidden-path
//! support.
//!
//! ### Semantic boundary (the bound is a real SPARQL construct, not an approx)
//!
//! The operator implements the *bounded* path `{m,k}` exactly — a well-defined
//! SPARQL 1.1 construct. It is NOT the unbounded `(p)+`/`(p)*` closure; a pair
//! connected only by a chain of length `> k` is (correctly, by definition of the
//! bounded form) absent. The differential oracle is the clear-text engine's
//! `eval_path` over the *union* store evaluating the **same bounded form**
//! (`p{m,k}`), so the equality is exact, not an approximation (design §1.2).
//!
//! ## Supported path forms (the acceptance matrix)
//!
//! - **sequence** `p1/p2/.../pk` — one fixed BGP chain ([`PathForm::Sequence`]).
//! - **exact** `(p){k}` — one fixed `k`-hop chain ([`BoundedRepetition`] `m==k`).
//! - **range** `(p){m,k}` — UNION of the exactly-`ℓ` chains for `ℓ in m..=k`.
//! - **reflexive** `(p){0,k}` — the `{1,k}` union PLUS the length-0 identity pairs
//!   `(x,x)` over the **declared public node domain** (design §2.3); the identity
//!   pairs appear exactly once after dedup. SPARQL 1.1 §18.4 `ZeroLengthPath` binds
//!   the zero-length match to every distinct subject/object term of the active
//!   graph — literals and object-only nodes included, predicate-only terms
//!   excluded — which is strictly wider than the endpoints of the path's own
//!   disclosed predicates, so the domain is a declared plan input of `Term`s
//!   ([`DisclosedEdges::with_node_domain`]) and a `min == 0` form without it is
//!   REFUSED rather than answered from a narrower domain.
//! - **alternation** `(p1|p2|...)` per hop — each hop position expands over its
//!   alternatives; a length-`ℓ` chain of an `a`-way alternation unrolls to `a^ℓ`
//!   fixed chains (design §2.4), all unioned and deduped. Because that count is
//!   exponential in the PUBLIC bound, the unroller refuses any repetition whose
//!   projected chain count exceeds [`MAX_UNROLL_CHAINS`] (a controlled
//!   `MpcError::Protocol`, checked BEFORE any allocation), so a large public
//!   `{m,k}`/alternation cannot become a CPU/memory denial-of-service. The per-chain
//!   HOP count is bounded first, by [`MAX_UNROLL_HOPS`]: a one-way step unrolls to
//!   `k - m + 1` chains however large `k` is, so the chain cap does not bind on it,
//!   while each chain of length `ℓ` is still an `ℓ`-hop join fold. Those two caps
//!   bound the unroll's SHAPE; what bounds a single chain's cost is that the fold
//!   projects to `(subject, reached)` and dedups after every hop, so its
//!   accumulator holds endpoint PAIRS (at most `|V|²`) and never one row per walk.
//!   See [`MAX_UNROLL_HOPS`] for what the caps do and do not promise.
//!
//! Composition: a [`BoundedRepetition`] is a repetition `{m,k}` of a single
//! [`PathStep`] (a named predicate or an alternation of named predicates); a
//! [`PathForm::Sequence`] is a fixed sequence of such forms each with its own
//! bound, distributed into the chain set. Endpoint-pair DEDUP is by the
//! `(a,b)` term pair (set semantics for path endpoints, design §2.2/§2.5) — the
//! realized length is NEVER part of the output (design §1.2 length-revealing
//! prohibition), so a pair reached by two different lengths appears once.

use crate::join::{DisclosedKeyJoin, GlobalJoin, JoinPlan};
use crate::partial::{HolderId, MpcError, PartialResult};
use oxrdf::{NamedNode, Term, Variable};
use std::collections::{BTreeMap, BTreeSet};

/// One hop of a bounded path: a single predicate, or an ALTERNATION of
/// predicates (`p1 | p2 | ...`). Each alternative is a disclosed global-IRI
/// predicate. A bare predicate is `alternatives.len() == 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStep {
    /// The predicate IRIs this hop may traverse (one = a plain `p`, many = an
    /// alternation `p1|p2|...`). Must be non-empty.
    pub alternatives: Vec<NamedNode>,
}

impl PathStep {
    /// A single-predicate hop `p`.
    pub fn predicate(p: NamedNode) -> Self {
        PathStep {
            alternatives: vec![p],
        }
    }

    /// An alternation hop `p1 | p2 | ...`.
    pub fn alternation(alts: impl IntoIterator<Item = NamedNode>) -> Self {
        PathStep {
            alternatives: alts.into_iter().collect(),
        }
    }
}

/// A bounded repetition of one [`PathStep`]: `(step){m,k}` with PUBLIC, finite
/// `m <= k`. This is the core repetition unit; [`PathForm`] composes it.
///
/// - `(p){k}` is `m == k` (exactly-`k`).
/// - `(p){m,k}` is the range.
/// - `(p){0,k}` (`m == 0`) adds the length-0 reflexive identity pairs (design
///   §2.3). `(p?)` is the special case `{0,1}`.
///
/// The bound is PUBLIC and part of the query plan; it is a stated parameter, not
/// a secret (design §1.2/§4.1). An OPEN upper bound (`{m,}`, `+`, `*`) is NOT
/// representable here by construction — `max` is a concrete `usize` — which is
/// the point: only the bounded, tractable slice lives in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedRepetition {
    /// The repeated step.
    pub step: PathStep,
    /// Minimum hop count `m` (`0` enables the reflexive identity pairs).
    pub min: usize,
    /// Maximum hop count `k` (PUBLIC, finite, `>= min`).
    pub max: usize,
}

/// A bounded property-path FORM the unroller understands. Either a single
/// bounded repetition, or a fixed sequence of bounded repetitions (`/`).
///
/// A pure sequence of single-predicate steps `p1/p2/.../pk` is
/// `Sequence(vec![{p1}{1,1}, {p2}{1,1}, ...])`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathForm {
    /// `(step){m,k}` — a bounded repetition of one (possibly alternation) step.
    Repetition(BoundedRepetition),
    /// `f1 / f2 / ...` — a fixed sequence of bounded forms, joined head-to-tail.
    /// The endpoint pairs are the relational composition of the sub-forms'
    /// pair relations.
    Sequence(Vec<PathForm>),
}

impl PathForm {
    /// Exactly-`k` repetition `(p){k}` of a single predicate.
    pub fn exact(p: NamedNode, k: usize) -> Self {
        PathForm::Repetition(BoundedRepetition {
            step: PathStep::predicate(p),
            min: k,
            max: k,
        })
    }

    /// Range repetition `(p){m,k}` of a single predicate.
    pub fn range(p: NamedNode, m: usize, k: usize) -> Self {
        PathForm::Repetition(BoundedRepetition {
            step: PathStep::predicate(p),
            min: m,
            max: k,
        })
    }

    /// Range repetition `(step){m,k}` of an arbitrary (possibly alternation) step.
    pub fn repeat_step(step: PathStep, m: usize, k: usize) -> Self {
        PathForm::Repetition(BoundedRepetition {
            step,
            min: m,
            max: k,
        })
    }

    /// A plain sequence of single predicates `p1/p2/.../pn` (each exactly one hop).
    pub fn sequence(preds: impl IntoIterator<Item = NamedNode>) -> Self {
        PathForm::Sequence(preds.into_iter().map(|p| PathForm::exact(p, 1)).collect())
    }
}

/// Maximum number of fixed-length BGP chains a single bounded repetition may
/// unroll to before we refuse the path with [`MpcError::Protocol`].
///
/// A repetition `(step){m,k}` over an `a`-way alternation unrolls to
/// `Σ_{ℓ=m..=k} a^ℓ` fixed chains (design §2.4). That sum is **exponential in `k`**
/// and **polynomial-of-degree-`k` in `a`**, so a modest-looking public bound — e.g.
/// a 4-way alternation with `k = 16` — already projects to billions of chains.
/// Each chain is then evaluated as a multi-hop join fold, so an unchecked unroll is
/// a CPU/memory denial-of-service on this **public** API surface (the reviewer's
/// `alternatives^length` blowup). Because the bound `k` and the alternation arity
/// are PUBLIC plan parameters (design §1.2/§4.1), the projected chain count is
/// computable in closed form WITHOUT touching any holder data — so we can reject an
/// over-cap path *before* allocating or evaluating anything.
///
/// The cap is a deliberately generous-but-finite ceiling: large enough that every
/// realistic disclosed-key bounded path (small `k`, small alternations) passes, far
/// below the point where the chain set would exhaust memory. It is a protocol
/// guard, not a tuning knob; a query that exceeds it is malformed/abusive for this
/// regime, not merely slow.
pub const MAX_UNROLL_CHAINS: u64 = 1 << 20; // 1_048_576

/// Largest per-chain HOP count a `(step){m,k}` repetition may unroll to, checked
/// before [`MAX_UNROLL_CHAINS`].
///
/// The chain cap alone does not bound the work, because the chain count is not the
/// only thing `k` grows. A ONE-way step (a plain predicate, no alternation) unrolls
/// to just `k - m + 1` chains however large `k` is, so the chain cap never binds on
/// it — yet chain `ℓ` is an `ℓ`-hop join fold, so the union evaluates `Σ_{ℓ=m..=k} ℓ`
/// joins and allocates an `ℓ`-long predicate sequence per chain. A single-predicate
/// `{0,k}` with a huge public `k` was therefore unbounded work that the chain cap
/// waved through. It also reached `projected_chain_count`, whose own `a^ℓ` loop
/// runs `ℓ` times per length, so even deciding to refuse grew with `k`.
///
/// Bounding the hop count first fixes both: with `a >= 2` alternatives the chain cap
/// already forces `k <= 20`, so this guard is what covers the one-way case, and it
/// keeps the projection's own loops short. Like the chain cap it is a protocol
/// guard, not a tuning knob: it sits far above any property path a query plans, and
/// a repetition that exceeds it is malformed for this regime rather than merely
/// slow.
///
/// ## What the two caps guarantee, and what they do not
///
/// They bound the SHAPE of the unroll, nothing else:
///
/// - [`MAX_UNROLL_CHAINS`] bounds how many fixed chains the union evaluates.
/// - `MAX_UNROLL_HOPS` bounds how long each of those chains is.
///
/// Per-chain cost is what makes those two enough. `eval_fixed_chain` projects to
/// `(subject, reached)` and dedups after EVERY hop, so one chain costs at most
/// `hops · |V| · |E|` and its accumulator holds at most `|V|²` rows. Without that
/// projection the accumulator would hold one row per WALK, which no cap on the
/// shape can bound — a 64-hop chain on a dense graph is exponential in the hop
/// count even though its ANSWER has at most `|V|²` rows.
///
/// So total work is bounded by `chains · hops · |V| · |E|`, and at both caps at
/// once that product is large: a near-cap alternation such as `(p|q){1,19}` is
/// about `2^20` chains of up to 19 hops each, which is deliberately still accepted
/// and is a heavy run, not a quick one. What the caps rule out is the UNBOUNDED
/// case, not an expensive one.
pub const MAX_UNROLL_HOPS: usize = 64;

/// Closed-form count of fixed-length chains a single `(step){m,k}` repetition
/// unrolls to: `Σ_{ℓ=m..=k} a^ℓ` where `a = alternatives.len()` (length-0 adds the
/// reflexive identity arm, which is NOT a chain — it is handled separately — so it
/// contributes nothing here). Returns `None` on `u64` overflow so the caller can
/// reject the path with a controlled error instead of risking a wrapping/huge
/// allocation.
fn projected_chain_count(alternatives: usize, min: usize, max: usize) -> Option<u64> {
    let a = alternatives as u64;
    let mut total: u64 = 0;
    for length in min..=max {
        if length == 0 {
            // The reflexive identity arm is not an unrolled chain.
            continue;
        }
        // a^length, checked.
        let mut term: u64 = 1;
        for _ in 0..length {
            term = term.checked_mul(a)?;
        }
        total = total.checked_add(term)?;
    }
    Some(total)
}

/// Reserved subject-end variable name the unroller projects onto (`?__pp_a`).
const SUBJECT_VAR: &str = "__pp_a";
/// Reserved object-end variable name the unroller projects onto (`?__pp_b`).
const OBJECT_VAR: &str = "__pp_b";
/// Prefix for the fresh intermediate join variables `?__pp_z{i}` between hops.
const INTERMEDIATE_PREFIX: &str = "__pp_z";

/// The disclosed edge-relations the unroller joins over. ONE entry per
/// **predicate IRI** that appears anywhere in the path: the set of `(subject,
/// object)` global-IRI pairs each holder disclosed for that predicate, already
/// merged into a single disclosed `(?__pp_edge_s, ?__pp_edge_o)` partial per
/// predicate (the union over holders — the federated edge set for that
/// predicate). Because the keys are disclosed global IRIs, building this set is a
/// plaintext union; NO MPC is involved.
///
/// This is the disclosed-key analogue of "the graph": the unroller never sees
/// raw holder graphs, only these per-predicate disclosed edge relations
/// (minimise data sharing, architecture §4.2). The `(a,b)` pairs are the headline
/// federation case: endpoints are disclosed global IRIs.
///
/// A zero-length arm (`{0,k}` / `p?` / a bounded `p*`) additionally needs the
/// PUBLIC node domain, which these per-predicate edge sets do NOT carry — see
/// [`DisclosedEdges::with_node_domain`]. That domain is a set of `Term`s and may
/// contain literals; only the EDGE endpoints are restricted to global IRIs.
#[derive(Clone)]
pub struct DisclosedEdges {
    /// `(predicate IRI string, disclosed (?__pp_edge_s, ?__pp_edge_o) edge partial)`.
    by_predicate: Vec<(String, PartialResult)>,
    /// The caller-declared PUBLIC node domain of the federation's active graph,
    /// if any. Deduped and canonically ordered; any `Term`, literals included.
    /// `None` = not declared, and a form with `min == 0` is then refused.
    node_domain: Option<Vec<Term>>,
}

/// `Debug` prints SIZES only — the per-predicate row counts and the declared
/// domain's size — not the terms.
///
/// Nothing here is secret: this is the disclosed-key regime, every edge endpoint
/// and every declared node is a term the federation published, and the sibling
/// [`crate::hidden_path::PredicatedEdges`] redacts for a reason this type does
/// not share. It is hygiene: a derived `Debug` on an edge set spills the whole
/// federated relation into any log line, panic message or `assert_eq!` failure
/// that happens to render one, which is noise at best. The sizes are what a
/// `Debug` is actually wanted for here.
impl std::fmt::Debug for DisclosedEdges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let by_predicate: Vec<String> = self
            .by_predicate
            .iter()
            .map(|(pred, part)| format!("{}: {} edge(s)", pred, part.rows.len()))
            .collect();
        f.debug_struct("DisclosedEdges")
            .field("by_predicate", &by_predicate)
            .field(
                "declared_node_domain",
                &match &self.node_domain {
                    Some(d) => format!("{} term(s)", d.len()),
                    None => "<none declared>".to_string(),
                },
            )
            .finish()
    }
}

impl DisclosedEdges {
    /// Build the per-predicate disclosed edge relations from each holder's
    /// disclosed edge partials.
    ///
    /// `holder_edges` is one `(predicate, partial)` per holder-disclosed edge
    /// set, where `partial.vars == [?subject_var, ?object_var]` (in that order) —
    /// i.e. exactly what a holder gets from
    /// `evaluate_local("SELECT ?s ?o WHERE { ?s <p> ?o }")`. Rows for the same
    /// predicate from different holders are UNIONED (the federated edge set).
    ///
    /// Soundness: the subject/object variable names of each partial may differ
    /// per holder; only the COLUMN ORDER (subject, object) is contractual, so we
    /// re-key every edge partial onto the reserved `(?__pp_edge_s, ?__pp_edge_o)`
    /// schema.
    pub fn from_holder_edges(
        holder_edges: impl IntoIterator<Item = (NamedNode, PartialResult)>,
    ) -> Result<Self, MpcError> {
        let edge_s = Variable::new_unchecked("__pp_edge_s");
        let edge_o = Variable::new_unchecked("__pp_edge_o");
        let mut by_predicate: Vec<(String, PartialResult)> = Vec::new();
        for (pred, part) in holder_edges {
            if part.vars.len() != 2 {
                return Err(MpcError::Protocol(format!(
                    "disclosed edge partial for <{}> must have exactly 2 columns \
                     (subject, object); got {} columns",
                    pred.as_str(),
                    part.vars.len()
                )));
            }
            // Soundness: the disclosed-key regime joins on GLOBAL IRIs. Both
            // endpoints of every disclosed edge MUST be NamedNodes. A blank node
            // would be unsound — blank-node labels are only document-scoped, so two
            // holders emitting the same label do NOT denote the same term, yet the
            // disclosed-key equi-join would treat equal labels as equal terms
            // (a false cross-holder join). A literal endpoint is outside the
            // global-IRI key regime entirely. Reject either at ingest (fail fast)
            // rather than silently producing unsound joins (design §2.1 DISCLOSED
            // regime boundary).
            for row in &part.rows {
                if row.len() != 2 {
                    return Err(MpcError::Protocol(format!(
                        "disclosed edge row for <{}> must have width 2; got {}",
                        pred.as_str(),
                        row.len()
                    )));
                }
                for (col, slot) in row.iter().enumerate() {
                    match slot {
                        Some(Term::NamedNode(_)) => {}
                        other => {
                            return Err(MpcError::Protocol(format!(
                                "disclosed edge for <{}> has a non-IRI endpoint in \
                                 column {col} ({other:?}); the disclosed-key regime \
                                 requires global-IRI (NamedNode) endpoints — blank \
                                 nodes are document-scoped (unsound to join across \
                                 holders) and literals are out of regime",
                                pred.as_str()
                            )));
                        }
                    }
                }
            }
            // Re-key onto the canonical (?__pp_edge_s, ?__pp_edge_o) schema so the
            // holder's chosen variable names cannot influence the join.
            let rekeyed = PartialResult {
                holder: part.holder.clone(),
                vars: vec![edge_s.clone(), edge_o.clone()],
                rows: part.rows.clone(),
            };
            let key = pred.as_str().to_string();
            match by_predicate.iter_mut().find(|(k, _)| *k == key) {
                Some((_, acc)) => acc.rows.extend(rekeyed.rows),
                None => by_predicate.push((key, rekeyed)),
            }
        }
        Ok(DisclosedEdges {
            by_predicate,
            node_domain: None,
        })
    }

    /// Declare the federation's PUBLIC node domain — every term that occurs as a
    /// subject or an object of any triple in the active graph — so a zero-length
    /// arm can be evaluated.
    ///
    /// ## Why this has to come from the caller
    ///
    /// SPARQL 1.1 §18.4 `ZeroLengthPath` binds the zero-length match to **every
    /// distinct subject or object term of the active graph** — not just the terms
    /// of the predicates the path traverses. That domain includes object-only
    /// nodes and **literals**, and excludes predicate-only terms, exactly as the
    /// clear-text engine's `graph_nodes` computes it. The per-predicate relations
    /// built by
    /// [`DisclosedEdges::from_holder_edges`] carry only the edges holders
    /// disclosed for the path's own predicates, so their endpoint terms are a
    /// SUBSET of that domain: a node occurring only in a triple of some other
    /// predicate (or of a predicate no holder disclosed) is absent. Drawing the
    /// identity pairs from the disclosed edges alone therefore dropped those
    /// `(x, x)` pairs and answered a `{0,k}` / `p?` form with fewer rows than the
    /// clear-text engine's `eval_path` — a wrong answer, silently.
    ///
    /// Widening the disclosure to recover the domain is not this module's call to
    /// make: the holders decide what they publish (minimise data sharing,
    /// architecture §4.2). So the domain is a declared, PUBLIC plan input, and
    /// without it a `min == 0` form is REFUSED ([`MpcError::Protocol`]) rather
    /// than answered from a narrower domain. A form with `min >= 1` never reads
    /// the domain and needs no declaration.
    ///
    /// ## The term type: `Term`, not `NamedNode`
    ///
    /// The domain is a set of [`Term`]s, **independent of the IRI-only EDGE
    /// regime**. The disclosed-key *edge* join requires global-IRI endpoints —
    /// blank node labels are document-scoped, so joining EDGES on them across
    /// holders would be unsound, and that check stays on
    /// [`DisclosedEdges::from_holder_edges`]. The engine's domain contains
    /// literals (every object of a triple whose object is a literal), so
    /// restricting the declaration to `NamedNode` made the true domain
    /// inexpressible and silently lost those `(x, x)` rows. A [`PartialResult`]
    /// row is `Vec<Option<Term>>`, so a literal endpoint needs nothing special to
    /// carry.
    ///
    /// A diagonal row is not join-free, though: inside a [`PathForm::Sequence`]
    /// the `(x, x)` rows ARE joined, by [`DisclosedKeyJoin`] on the term itself.
    /// That is sound for a non-IRI term precisely BECAUSE the edges are IRI-only:
    /// a literal or blank-node `(x, x)` row can only ever meet another diagonal
    /// row built from the same declared term, never an edge endpoint, so no
    /// cross-holder identification is made on a document-scoped or
    /// non-global key. The declared term is the caller's own, single,
    /// federation-wide name for that node.
    ///
    /// ## The precondition — what the caller is trusted for
    ///
    /// The caller is trusted to supply **exactly** the selected active graph's
    /// subject/object domain. The only thing this method checks is that the
    /// declaration COVERS the endpoints of the disclosed edges (those are terms of
    /// the active graph by construction, so a declaration missing one is
    /// inconsistent with the edges it accompanies, and is refused with
    /// [`MpcError::Protocol`]). **Neither completeness nor the absence of extra
    /// nodes is verified:** an omission loses `(x, x)` answers the engine returns,
    /// and an extra node invents an `(x, x)` answer the engine does not. Both are
    /// wrong answers this method cannot detect.
    ///
    /// ### Reproducing the engine's self-match for an ABSENT bound endpoint
    ///
    /// The engine matches a **bound** endpoint against itself even when that
    /// constant occurs in no triple (`sparq-engine`'s `exec/bgp.rs` skips the
    /// node-membership check for a concrete term). A caller can reproduce that by
    /// including the constant in the declared domain — no API change is needed.
    ///
    /// **This is correct only for a SINGLE repetition whose bound endpoint is that
    /// constant, declared per query.** It is a deliberate per-query WIDENING of the
    /// domain, taken with its consequence stated, not an exception to the
    /// "exactly the domain" precondition above — the widened set is no longer the
    /// active graph's domain, and every arm of the form sees it.
    ///
    /// So it OVER-answers as soon as two or more NULLABLE arms are chained in a
    /// [`PathForm::Sequence`], because the Sequence's midpoint ranges over the
    /// declared domain. Concretely, on `ex:a ex:p ex:b . ex:b ex:q ex:c` with the
    /// active nodes plus `ex:absent` declared, `Sequence[p{0,1}, q{0,1}]` yields
    /// `(absent, absent)` — it composes the two diagonals through the midpoint
    /// `absent` — while the engine's `ex:absent ex:p?/ex:q? ?b` returns NOTHING:
    /// its inner `q?` arm sees a VARIABLE midpoint, so that arm ranges over the
    /// graph's own nodes, which do not include `ex:absent`. Pinned by
    /// `an_absent_constant_over_answers_a_nested_nullable_sequence`.
    ///
    /// The whole HIDDEN regime is unaffected: [`crate::hidden_path`] evaluates a
    /// single repetition and has no Sequence form at all.
    ///
    /// Duplicates are collapsed, and the stored order is canonical, so the
    /// identity pairs do not depend on the order the caller listed its nodes in.
    pub fn with_node_domain(
        mut self,
        nodes: impl IntoIterator<Item = Term>,
    ) -> Result<Self, MpcError> {
        let mut declared: BTreeMap<String, Term> = BTreeMap::new();
        for n in nodes {
            declared.entry(format!("{:?}", n)).or_insert(n);
        }
        // Report only HOW MANY endpoints are uncovered: the count is enough to act
        // on, and the message stays free of term values.
        let mut missing: BTreeSet<String> = BTreeSet::new();
        for (_, part) in &self.by_predicate {
            for row in &part.rows {
                for slot in row.iter().flatten() {
                    let key = format!("{:?}", slot);
                    if !declared.contains_key(&key) {
                        missing.insert(key);
                    }
                }
            }
        }
        if !missing.is_empty() {
            // VALUE- AND COUNT-FREE. An earlier version named how many endpoints
            // were uncovered and how many terms were declared; both are statistics
            // over the disclosed edges, and a caller could probe one candidate term
            // at a time and read the count as a membership answer. The text now
            // carries neither.
            return Err(MpcError::Protocol(
                "the declared node domain does not cover the edge endpoints — every disclosed \
                 endpoint is a term of the active graph, so a zero-length arm over this \
                 domain would under-answer; declare the full subject/object domain of the \
                 active graph"
                    .into(),
            ));
        }
        self.node_domain = Some(declared.into_values().collect());
        Ok(self)
    }

    /// The disclosed edge relation for one predicate, projected onto a chosen
    /// `(subject_var, object_var)` schema (so a hop can be slotted into a chain
    /// at any position). Returns an EMPTY relation (no rows) for a predicate with
    /// no disclosed edges — an absent predicate simply contributes nothing, which
    /// is exactly how the clear-text engine treats it.
    fn relation_for(
        &self,
        pred: &NamedNode,
        subject: &Variable,
        object: &Variable,
    ) -> PartialResult {
        let rows = self
            .by_predicate
            .iter()
            .find(|(k, _)| k == pred.as_str())
            .map(|(_, p)| p.rows.clone())
            .unwrap_or_default();
        PartialResult {
            holder: HolderId::new("federation"),
            vars: vec![subject.clone(), object.clone()],
            rows,
        }
    }

    /// The declared PUBLIC node domain over which the reflexive (length-0)
    /// identity pairs `(x,x)` are added for `{0,k}` (design §2.3; the analogue of
    /// the clear-text engine's `graph_nodes`, which ranges over the WHOLE active
    /// graph).
    ///
    /// Fail-closed when the caller declared no domain: the disclosed per-predicate
    /// edge sets do not carry it, and the endpoint terms they do carry are only a
    /// subset of it, so there is nothing sound to fall back on. See
    /// [`DisclosedEdges::with_node_domain`].
    fn node_domain(&self) -> Result<&[Term], MpcError> {
        self.node_domain.as_deref().ok_or_else(|| {
            MpcError::Protocol(
                "bounded path with min == 0 (the zero-length arm of {0,k} / p? / a bounded p*) \
                 needs the federation's PUBLIC node domain, which the disclosed per-predicate \
                 edge sets do not carry: SPARQL 1.1 §18.4 binds the zero-length match to \
                 every distinct SUBJECT or OBJECT term of the active graph, including terms \
                 occurring only in triples of \
                 predicates this path does not traverse or that no holder disclosed. Refusing \
                 rather than drawing the identity pairs from the disclosed endpoints alone, \
                 which would silently under-answer. Declare the domain with \
                 DisclosedEdges::with_node_domain, or use a form with min >= 1."
                    .into(),
            )
        })
    }
}

/// Evaluate a bounded property-path `?a <form> ?b` over DISCLOSED edge relations
/// and return the deduped set of endpoint pairs `(a, b)` as a [`PartialResult`]
/// with `vars == [?__pp_a, ?__pp_b]`.
///
/// This is the increment-#1 deliverable: a CRYPTO-FREE unroll-and-fold. No MPC
/// primitive runs (no secret sharing, no `secure_equal`, no `CommCounter` is
/// touched) — every EDGE hop is a [`DisclosedKeyJoin`] over disclosed global IRIs,
/// exactly as the `differential_three_holder_chain_equals_union` test does for a
/// single chain. The realized hop length is not separately opened or encoded in
/// the result (design §1.2).
///
/// Regime: DISCLOSED-KEY ONLY — every key EDGES are joined on, and so every
/// chain intermediate, is a disclosed global IRI. A result endpoint or a Sequence
/// midpoint may additionally be a literal or blank node contributed by a
/// zero-length arm over the declared node domain. The HIDDEN-intermediate regime
/// is sq-py8h.2 and is NOT handled here.
pub fn eval_bounded_path_disclosed(
    edges: &DisclosedEdges,
    form: &PathForm,
) -> Result<PartialResult, MpcError> {
    let subject = Variable::new_unchecked(SUBJECT_VAR);
    let object = Variable::new_unchecked(OBJECT_VAR);

    // The pair RELATION over the canonical endpoint vars (?__pp_a, ?__pp_b),
    // computed by the unroll; then deduped to a set of endpoint pairs.
    // One counter for the whole evaluation, so every nested sequence gets its own
    // intermediate-variable namespace (see `eval_sequence`).
    let mut scope = 0usize;
    let relation = eval_form_relation(edges, form, &subject, &object, &mut scope)?;
    Ok(dedup_endpoint_pairs(relation, &subject, &object))
}

/// Evaluate one [`PathForm`] into its `(subject, object)` pair relation
/// (NOT yet deduped — the caller dedups once at the top). All joins are
/// disclosed-key folds; the construction is the design §2 unroll.
fn eval_form_relation(
    edges: &DisclosedEdges,
    form: &PathForm,
    subject: &Variable,
    object: &Variable,
    scope: &mut usize,
) -> Result<PartialResult, MpcError> {
    match form {
        // A repetition's own hop variables are `__pp_z{i}`, a different namespace
        // from a sequence's `__pp_zs{scope}_{i}`, and repetitions do not nest (a
        // step carries predicates, not sub-forms), so they need no scope of their
        // own.
        PathForm::Repetition(rep) => eval_repetition(edges, rep, subject, object),
        PathForm::Sequence(parts) => eval_sequence(edges, parts, subject, object, scope),
    }
}

/// `(step){m,k}` → UNION over `ℓ in m..=k` of the exactly-`ℓ` chains, plus the
/// reflexive identity pairs when `m == 0` (design §2.2/§2.3). Each exactly-`ℓ`
/// chain is a fold of [`DisclosedKeyJoin`] over `ℓ` copies of the step's edge
/// relation through fresh intermediate vars; alternation expands each hop over
/// its alternatives (design §2.4).
fn eval_repetition(
    edges: &DisclosedEdges,
    rep: &BoundedRepetition,
    subject: &Variable,
    object: &Variable,
) -> Result<PartialResult, MpcError> {
    if rep.min > rep.max {
        return Err(MpcError::Protocol(format!(
            "bounded path {{{},{}}} has min > max",
            rep.min, rep.max
        )));
    }
    if rep.step.alternatives.is_empty() {
        return Err(MpcError::Protocol(
            "bounded path step has no predicates (empty alternation)".into(),
        ));
    }

    // UNROLL-SIZE GUARD (sq-py8h.1, reviewer thread): `(step){m,k}` over an
    // `a`-way alternation unrolls to `Σ_{ℓ=m..=k} a^ℓ` fixed chains, which is
    // exponential in the PUBLIC bound `k`. Compute that projected chain count in
    // closed form FIRST (no holder data touched, no allocation) and reject an
    // over-cap path with a controlled `MpcError::Protocol` *before* generating or
    // evaluating any chain — otherwise a large public `{m,k}`/alternation is a
    // CPU/memory DoS (huge `Vec` growth, or a `usize` overflow in a `with_capacity`)
    // on this public API surface.
    // HOP-COUNT GUARD: bound the unroll work before planning it. The chain cap
    // below is computed per chain and does not bind on a one-way step, whose chain
    // count is `k - m + 1` whatever `k` is, so without this a single-predicate
    // `{m,k}` would fold `Σ_{ℓ=m..=k} ℓ` joins. Checking it FIRST also keeps
    // `projected_chain_count`'s per-length loop short. No holder data is touched and
    // nothing is allocated.
    if rep.max > MAX_UNROLL_HOPS {
        return Err(MpcError::Protocol(format!(
            "bounded path {{{},{}}} exceeds the MAX_UNROLL_HOPS cap of {} hops per chain \
             — refusing to evaluate (each chain of length L is an L-hop join fold, so \
             the unrolled work grows with the public bound even for a one-way step)",
            rep.min, rep.max, MAX_UNROLL_HOPS
        )));
    }
    let projected = projected_chain_count(rep.step.alternatives.len(), rep.min, rep.max);
    match projected {
        Some(count) if count <= MAX_UNROLL_CHAINS => {}
        _ => {
            return Err(MpcError::Protocol(format!(
                "bounded path {{{},{}}} over a {}-way alternation would unroll to {} \
                 fixed chains, exceeding the MAX_UNROLL_CHAINS cap of {} — refusing \
                 to evaluate (would be a CPU/memory denial-of-service)",
                rep.min,
                rep.max,
                rep.step.alternatives.len(),
                projected
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "an amount that overflows u64".to_string()),
                MAX_UNROLL_CHAINS,
            )));
        }
    }

    // Accumulate the union of all chain lengths as raw (a,b) rows; dedup happens
    // once at the top.
    let mut union_rows: Vec<Vec<Option<Term>>> = Vec::new();

    for length in rep.min..=rep.max {
        if length == 0 {
            // Length-0: the reflexive identity pairs (x,x) over the DECLARED
            // public node domain (design §2.3; SPARQL 1.1 §18.4). Refused when no
            // domain was declared — the disclosed edges' endpoints are only a
            // subset of it.
            for node in edges.node_domain()? {
                union_rows.push(vec![Some(node.clone()), Some(node.clone())]);
            }
            continue;
        }
        // exactly-`length` chains: expand the alternation across the `length`
        // hop positions → `alternatives^length` fixed BGP chains.
        for chain in alternation_chains(&rep.step, length) {
            let chain_rel = eval_fixed_chain(edges, &chain, subject, object)?;
            union_rows.extend(chain_rel.rows);
        }
    }

    Ok(PartialResult {
        holder: HolderId::new("federation"),
        vars: vec![subject.clone(), object.clone()],
        rows: union_rows,
    })
}

/// Every fixed predicate sequence of `length` hops obtained by choosing, at each
/// position, one of the step's alternatives (the §2.4 distribution). For a
/// single-predicate step this is just the one `length`-long sequence.
///
/// The caller caps the chain COUNT before reaching here ([`MAX_UNROLL_HOPS`] on the
/// length, then [`MAX_UNROLL_CHAINS`] on `Σ_ℓ a^ℓ`), so the number of sequences is
/// bounded. Nothing bounds their WIDTHS — a predicate IRI's length is unbounded —
/// and width caps are follow-up work, not a property this path provides.
fn alternation_chains(step: &PathStep, length: usize) -> Vec<Vec<NamedNode>> {
    let mut chains: Vec<Vec<NamedNode>> = vec![Vec::new()];
    for _ in 0..length {
        // Grow the chain set WITHOUT a `chains.len() * alternatives.len()`
        // `with_capacity`: that product is `alternatives^length`, which can
        // overflow `usize` or request an enormous allocation for large unrolls,
        // turning a should-be-controlled protocol error into a panic/abort on a
        // public API surface. Let the `Vec` grow amortised instead. The genuine
        // unroll-size limit is enforced upstream by `eval_repetition`'s
        // `MAX_UNROLL_CHAINS` guard (which runs BEFORE this function is reached),
        // so by the time we get here the total chain count is provably bounded.
        let mut next: Vec<Vec<NamedNode>> = Vec::new();
        for prefix in &chains {
            for alt in &step.alternatives {
                let mut extended = prefix.clone();
                extended.push(alt.clone());
                next.push(extended);
            }
        }
        chains = next;
    }
    chains
}

/// A fixed predicate chain `p_1 / p_2 / ... / p_n` (n = chain.len()) →
/// `?a p_1 ?z1 . ?z1 p_2 ?z2 . ... ?z_{n-1} p_n ?b`, evaluated as a left-to-right
/// fold of [`DisclosedKeyJoin`] on the intermediate vars, projected onto
/// `(subject, object)`. This is the §2.1 core (DISCLOSED regime): a fold of
/// crypto-free equi-joins — the same shape as the existing 3-pattern chain test.
fn eval_fixed_chain(
    edges: &DisclosedEdges,
    chain: &[NamedNode],
    subject: &Variable,
    object: &Variable,
) -> Result<PartialResult, MpcError> {
    debug_assert!(
        !chain.is_empty(),
        "exactly-0 chains handled by reflexive arm"
    );
    let n = chain.len();

    // Hop endpoint vars: ?__pp_a, ?__pp_z1, ..., ?__pp_z{n-1}, ?__pp_b.
    let hop_var = |i: usize| -> Variable {
        if i == 0 {
            subject.clone()
        } else if i == n {
            object.clone()
        } else {
            Variable::new_unchecked(format!("{INTERMEDIATE_PREFIX}{i}"))
        }
    };

    // First hop's edge relation: (?__pp_a, ?z1).
    let mut acc = edges.relation_for(&chain[0], &hop_var(0), &hop_var(1));

    // Fold remaining hops via DisclosedKeyJoin on the shared intermediate var,
    // collapsing the accumulator to `(subject, reached)` after EVERY hop.
    //
    // Carrying the intermediate columns to the end instead would make the
    // accumulator hold one row per distinct WALK, which grows as the number of
    // paths, not as the number of endpoint pairs — exponential in the chain length
    // on a dense graph, even though the projected result has at most `|V|²` rows.
    // Projecting and deduping per hop keeps the accumulator at most `|V|²` rows and
    // each hop's join at most `|V|·|E|`, and the RESULT is unchanged: only the
    // endpoints are ever returned (this function's one caller unions endpoint
    // pairs), and endpoint reachability is a set — two walks reaching the same
    // `(subject, reached)` pair continue identically.
    let join = DisclosedKeyJoin::new();
    for (idx, pred) in chain.iter().enumerate().skip(1) {
        let next = edges.relation_for(pred, &hop_var(idx), &hop_var(idx + 1));
        let join_var = hop_var(idx); // the shared intermediate this hop joins on
        let plan = JoinPlan {
            join_var,
            key_disclosed: true,
        };
        let joined = join.join(&[acc, next], &plan)?;
        let reached = hop_var(idx + 1);
        acc = dedup_endpoint_pairs(
            project_endpoints(&joined, subject, &reached)?,
            subject,
            &reached,
        );
    }

    // Project onto (subject, object), dropping any intermediate column. After the
    // fold above the accumulator is already `(subject, object)`; this also covers
    // the single-hop case, which never entered the loop.
    project_endpoints(&acc, subject, object)
}

/// `f1 / f2 / ... / fn` → relational composition of the sub-forms' pair
/// relations, folded left-to-right via [`DisclosedKeyJoin`] on a fresh
/// connecting variable. Each sub-form is evaluated onto `(prev_end, this_end)`
/// then joined on the shared middle var (design §2.1, generalised so a sequence
/// element may itself be a bounded repetition or alternation).
fn eval_sequence(
    edges: &DisclosedEdges,
    parts: &[PathForm],
    subject: &Variable,
    object: &Variable,
    scope: &mut usize,
) -> Result<PartialResult, MpcError> {
    if parts.is_empty() {
        return Err(MpcError::Protocol("empty path sequence".into()));
    }
    if parts.len() == 1 {
        return eval_form_relation(edges, &parts[0], subject, object, scope);
    }

    // Claim a scope id for THIS call before recursing, so no two sequence calls in
    // one evaluation share an intermediate name. Every level used to name its
    // connecting variables `__pp_zseq1`, `__pp_zseq2`, ... so a sequence nested in
    // a sequence reused the names of the level around it: the inner call's first
    // connecting variable was `__pp_zseq1`, which was also the outer mid variable
    // it had been handed as its SUBJECT. Evaluating a sub-form onto
    // `(__pp_zseq1, __pp_zseq1)` constrains both endpoints to be equal, which
    // silently drops the pairs that reach the inner sequence through distinct
    // endpoints — a wrong answer, not an error.
    *scope += 1;
    let my_scope = *scope;
    let mid_var = move |i: usize| {
        Variable::new_unchecked(format!("{}s{}_{}", INTERMEDIATE_PREFIX, my_scope, i))
    };

    // First sub-form onto (?__pp_a, mid1).
    let first_end = mid_var(1);
    let mut acc = eval_form_relation(edges, &parts[0], subject, &first_end, scope)?;
    let mut prev_end = first_end;

    let join = DisclosedKeyJoin::new();
    for (idx, part) in parts.iter().enumerate().skip(1) {
        let last = idx == parts.len() - 1;
        let this_end = if last {
            object.clone()
        } else {
            mid_var(idx + 1)
        };
        let part_rel = eval_form_relation(edges, part, &prev_end, &this_end, scope)?;
        // Dedup each sub-form's relation before joining so a sub-form's internal
        // multiplicity (e.g. two lengths reaching the same mid pair) does not
        // inflate the composition — endpoint relations are sets.
        let part_rel = dedup_endpoint_pairs(part_rel, &prev_end, &this_end);
        let acc_dedup = dedup_endpoint_pairs(acc, subject, &prev_end);
        let plan = JoinPlan {
            join_var: prev_end.clone(),
            key_disclosed: true,
        };
        acc = join.join(&[acc_dedup, part_rel], &plan)?;
        acc = project_endpoints(&acc, subject, &this_end)?;
        prev_end = this_end;
    }

    project_endpoints(&acc, subject, object)
}

/// Project a partial onto exactly `[subject, object]`, dropping every other
/// (intermediate) column. Both endpoint vars MUST be present: this is an
/// internal-invariant projection (the unroller always constructs relations that
/// carry the endpoint columns it asks to project), so a missing endpoint var
/// indicates a logic/protocol violation inside this module — NOT a "no results"
/// situation. We FAIL FAST with a Protocol error rather than silently returning
/// an empty relation, which would mask the bug by turning it into "no path
/// found" and undermine soundness reasoning.
fn project_endpoints(
    part: &PartialResult,
    subject: &Variable,
    object: &Variable,
) -> Result<PartialResult, MpcError> {
    let s_idx = part.vars.iter().position(|v| v == subject);
    let o_idx = part.vars.iter().position(|v| v == object);
    let (si, oi) = match (s_idx, o_idx) {
        (Some(si), Some(oi)) => (si, oi),
        _ => {
            return Err(MpcError::Protocol(format!(
                "project_endpoints: endpoint var(s) missing from relation schema \
                 (subject {subject:?} present: {}, object {object:?} present: {}; \
                 actual vars: {:?}) — internal unroll invariant violated",
                s_idx.is_some(),
                o_idx.is_some(),
                part.vars
            )));
        }
    };
    let rows = part
        .rows
        .iter()
        .map(|r| vec![r[si].clone(), r[oi].clone()])
        .collect();
    Ok(PartialResult {
        holder: HolderId::new("federation"),
        vars: vec![subject.clone(), object.clone()],
        rows,
    })
}

/// DEDUP the endpoint pairs to a SET (design §2.2/§2.5): each distinct `(a,b)`
/// term pair appears exactly once, regardless of how many chain lengths or
/// alternation branches reached it. The realized length is not part of the row,
/// so length is not separately opened or encoded in the result (design §1.2).
/// Output is sorted canonically so
/// the disclosed multiset is order-independent.
fn dedup_endpoint_pairs(
    part: PartialResult,
    subject: &Variable,
    object: &Variable,
) -> PartialResult {
    // Compute each row's canonical key ONCE (one allocation per row), then use it
    // for both dedup and ordering. The previous `sort_by` re-`format!`-ed both
    // operands on every comparison — O(n log n) string allocations — which gets
    // expensive on large result sets; keying first is O(n) allocations.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut keyed: Vec<(String, Vec<Option<Term>>)> = Vec::new();
    for row in part.rows {
        let key = format!("{row:?}");
        if seen.insert(key.clone()) {
            keyed.push((key, row));
        }
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    let rows: Vec<Vec<Option<Term>>> = keyed.into_iter().map(|(_, row)| row).collect();
    PartialResult {
        holder: HolderId::new("federation"),
        vars: vec![subject.clone(), object.clone()],
        rows,
    }
}

#[cfg(test)]
mod tests;
