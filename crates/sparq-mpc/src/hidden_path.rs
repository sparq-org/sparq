// sq-py8h.2 — HIDDEN-intermediate fixed exactly-`k` property-path chain
// (the cryptographic core of the bounded property-path operator).
//! Bounded property-path over a **secret graph**: the HIDDEN-intermediate fixed
//! exactly-`k` chain (`?a (p){k} ?b`) where the intermediate nodes `?z_1 … ?z_{k−1}`
//! are **never opened**.
//!
//! ## What this is (and how it differs from the disclosed-key sibling)
//!
//! This resolves **increment #2** of the bounded property-path design
//! (`research/mpc-bounded-property-path-design.md` §2.1 HIDDEN regime, §2.6, §6
//! step 2) — the **cryptographic core** the disclosed-key module
//! ([`crate::bounded_path`]) explicitly deferred. There the join keys are disclosed
//! global IRIs and the whole chain is a crypto-free fold of equi-joins. HERE the
//! intermediate comparisons run on secret-shared `F_p` key sharings whose values are
//! not explicitly opened: a chain `a → z_1 → … → z_{k−1} → b` exists iff there is a
//! sequence of `k` edges whose join points line up, and each "lines up" test is
//! decided as a **secret-shared match bit the protocol never opens** — so no
//! intermediate node `z_i`, and no per-hop / per-pair match pattern, is ever
//! reconstructed by the arithmetic. That is not a confidentiality claim against the
//! simulator host or an observer given the clear edge relation; see the disclosure
//! list below for what the construction does make readable.
//!
//! ## The construction (design §2.1 / §2.6, realised against landed primitives)
//!
//! The federated edge set `E` is the union of every holder's `(subject, object)`
//! edges for the path predicate, each endpoint carried as `(private F_p key,
//! disclosed endpoint term)`. (The endpoint *terms* are the disclosed RESULT of the
//! path — convention #4: the *intermediate join values* are the part the
//! arithmetic keeps secret-shared. What the construction as a whole makes
//! readable is wider; the disclosure list below is the contract. The
//! candidate edge-tuple COUNT `|E|^k` is public, the standard MPC assumption.)
//!
//! An exactly-`k` chain is a `k`-tuple of edges `(e_1, …, e_k)` that is "connected":
//!
//! ```text
//! connected(e_1,…,e_k) = AND_{i=1..k-1}  [ e_i.object_key == e_{i+1}.subject_key ]
//! ```
//!
//! Each internal equality is computed with [`crate::compare::secure_equal_to_bit`]
//! — a bit-decomposition equality whose 0/1 verdict is a fresh degree-`t`
//! **secret-shared** sharing the protocol never opens — so the per-pair and
//! per-hop match pattern is kept out of the arithmetic *at the decision*, not
//! merely at the output. That is a property of the protocol, not a
//! confidentiality claim: see the disclosure list below for what the
//! construction does make readable. The
//! `k − 1` internal-hop bits are AND-ed into a single per-tuple **chain bit** by
//! one [`crate::shamir::mul_shares_raw`] + one
//! [`crate::shamir::degree_reduce_through`] per AND — the BGW reshare-and-
//! recombine round (sq-dvuc, the keystone this bead depends on) that lets a
//! degree-`2t` product be reshared back to degree `t` so the next AND can chain.
//! For `k = 1` every edge is a trivially-connected length-1 chain, so the chain bit
//! is the constant shared `1` (no internal hop).
//!
//! Each tuple is a candidate whose endpoint pair `(a, b) = (e_1.subject,
//! e_k.object)` is the disclosed payload. **Set semantics (design §2.2/§2.5):** a
//! pair reachable by several distinct edge-tuples must appear ONCE, with the
//! realized chain *never* revealed. We therefore GROUP tuples by their disclosed
//! endpoint pair and **OR-fold** their chain bits into one connected-bit per
//! distinct `(a, b)` (`OR(x,y) = 1 − (1−x)(1−y)`, one more `mul_shares_raw` +
//! `degree_reduce_through`). Grouping is over the DISCLOSED endpoint terms — never
//! over any secret intermediate key. It is not free of disclosure, though: the
//! grouping key is the endpoint pair of EVERY enumerated tuple, matched or not, so
//! anything supplied with those ordered plaintext terms — the simulator host here,
//! and every evaluating party in a distributed implementation that hands them out —
//! can determine the ordered candidate pair sequence and where each tuple folds.
//! See the disclosure list below.
//!
//! The one connected-bit per distinct endpoint pair becomes a
//! [`crate::oblivious_join::Candidate`] with a
//! [`crate::oblivious_join::MatchBit::SecretShared`] selector, fed verbatim into the
//! landed oblivious output path
//! ([`crate::oblivious_join::oblivious_set_output`], sq-jnkm): the secret bit drives
//! an oblivious select (one product, no open), the slots are shuffled, and exactly a
//! public padded bound `B` of slots is revealed. The per-pair match bit is never
//! opened anywhere; only the `B` shuffled output tags are opened (classifying a slot
//! as a real endpoint row or a dummy AFTER the permutation destroyed
//! position→candidate linkage).
//!
//! ## What stays hidden / what is disclosed (state it precisely)
//!
//! - **Not explicitly opened:** every intermediate node key `z_i` (never
//!   reconstructed); every per-hop and per-pair match bit (secret-shared); which
//!   tuples connected a pair and the realized chain length (the OR-fold collapses
//!   multiplicity). "Not opened" is a statement about the arithmetic protocol, not
//!   a confidentiality claim: it does not hide node identities or connectivity from
//!   the simulator host, or from an observer supplied the clear edge terms and their
//!   associations — under the documented consistent encoding
//!   ([`crate::term_encode::encode_term`]) those terms determine hop equality and
//!   chain connectivity. The NUMBER of distinct endpoint pairs is NOT in this list
//!   either — see the schedule bullet below.
//! - **Disclosed by the SCHEDULE (every successful run):** the OR-fold POSITIONS —
//!   which tuple positions repeat an endpoint pair — and therefore the distinct
//!   endpoint-pair count `D`. Who learns them:
//!   - **In the model as built**, a single process simulates every party. The
//!     evaluator that groups does so on plaintext `Term`s, so it sees the ORDERED
//!     candidate pair sequence, unmatched pairs included — strictly more than the
//!     result, which names the MATCHED pairs only.
//!   - **In a distributed implementation** that supplies every evaluating party
//!     with the ordered plaintext endpoint terms, or with this construction's
//!     data-dependent accumulator wiring, every such party can determine the fold
//!     positions. This crate implements a single-process simulation and does not
//!     establish a per-party input or wiring-distribution boundary.
//!   - **Over a transport**, the ORs are not batched — each is one
//!     `mul_shares_raw` followed by one `degree_reduce_through`, inline in the
//!     enumeration. A sequential transport that exposes identifiable tuple and
//!     OR-gate boundaries would expose the fold positions; there is no
//!     hidden-path transport establishing which boundaries a network observer
//!     receives.
//!   - Independently of any of that, each repeat costs one extra multiplication,
//!     so the multiplication COUNT carries `D`, the public tuple total `T` being
//!     known: `T − D` ORs over the `T` tuples, or `T + N₀ − D` when a bounded form
//!     has `min == 0` and folds in one entry per node of the `N₀`-node DECLARED
//!     public node domain, plus the output stage's padding from `D` up to `B`. `T`
//!     is `|E|^k` for the exactly-`k` chain, and for a bounded form
//!     `T = Σ_{ℓ=max(min,1)}^{max} a^ℓ·|E|^ℓ` (empty sum = 0) with `a` the
//!     alternation width; length-zero entries are counted separately by `N₀`.
//!
//!   These are the views this module can account for; it does NOT claim a complete
//!   enumeration of what an arbitrary party or an arbitrary transport learns.
//!   Hiding `D` would need oblivious grouping with a schedule uniform in the edge
//!   count, which is NOT built. Two precisions: "every successful run" is meant
//!   literally — a refusal from `check_params` or from the public half of the bound
//!   contract happens before any candidate exists, and a tight-bound refusal runs
//!   the data-dependent projection but no build, so neither produces a draw
//!   schedule to read; and the padding also discloses the uniform payload WIDTH
//!   (next bullet).
//! - **Disclosed: the uniform payload width.** The output stage packs every slot
//!   to one width — the longest encoding over ALL candidates, the unmatched ones
//!   included, rounded up to whole limbs — and that width is the per-slot limb
//!   count in the opens and one multiplication per limb per slot, so it is visible
//!   to the parties and it drives the per-slot work. An unmatched candidate with a
//!   long endpoint term therefore widens every slot. A cap on that width is
//!   follow-up work, as [`crate::bounded_path`] also records; this stage provides
//!   none.
//! - **Disclosed by a REFUSAL:** that a parameter or bound was rejected, and which
//!   one. The output-bound check is taken on [`public_candidate_pair_bound`] /
//!   [`public_bounded_candidate_pair_bound`] — functions of `|E|` and the public
//!   hop counts — whenever `B` reaches them, and the exact-count projection is not
//!   run on that branch, so `D` stays out of the DECISION (the run itself still
//!   materialises `D` candidates, as the schedule above says). A caller that
//!   chooses a `B` BELOW the public bound puts `D` in the RESULT of the call as
//!   well: that branch accepts exactly
//!   when `D <= B`, and a refusal draws nothing, so repeated calls with different
//!   `B` recover `D` exactly by binary search. Given the schedule above that is no
//!   new disclosure, but the public-only error text must not be read as hiding it.
//!   Range refusals name the operand POSITION, never its value.
//! - **Disclosed:** the public bound `k`; the public candidate-tuple count `|E|^k`
//!   (standard MPC assumption); the endpoint TERMS of the matched pairs (the result
//!   the recipient asked for); the public padded bound `B`. The true result
//!   cardinality is bounded by `B`, and whether it is also disclosed depends on the
//!   output mode: this stage uses the ALL-PARTY mode, where every compute party
//!   opens the `B` tags and therefore learns the exact count; the recipient-only
//!   mode ([`crate::oblivious_join::oblivious_set_output_to_recipient`]) keeps it out
//!   of the compute parties' transcripts.
//!
//! ## Security tier (HONESTY — privacy-claims gate, cite sq-qhy4)
//!
//! Honest-majority, semi-honest ONLY — inherits the [`crate::shamir::ShamirBackend`]
//! / [`crate::compare`] model and adds no new assumption. Every multiplication
//! routes through [`crate::shamir::degree_reduce_through`], whose reshare has
//! no in-protocol check that a deviating party reshared honestly, so this is **NOT**
//! maliciously secure (the same degree-`2t`-at-`n=2t+1` residual the rest of the
//! backend documents; sq-6d6g / sq-km34 IT-MAC is the named fix). The
//! per-intermediate / per-pair confidentiality this closes is a CONFIDENTIALITY
//! axis, orthogonal to malicious security; external soundness sign-off is still
//! pending (sq-qhy4). No guarantee beyond the documented semi-honest model is
//! claimed.
//!
//! ## Scope boundary (do not over-read)
//!
//! This is the **exactly-`k`** chain ONLY (`?a (p){k} ?b`, `k >= 1`). The bounded
//! `{1,k}` / `{0,k}` / `p?` union + reflexive diagonal + alternation, with the
//! cross-length OR-fold dedup, is the sibling increment **sq-py8h.3** (it builds the
//! union of exactly-`ℓ` chains ON TOP of this). DISTINCT over genuinely-secret
//! endpoint pairs (collapsing duplicate *secret* `(a,b)`) is the gated piece
//! **sq-py8h.4** (needs the secure-sort comparator). Planner guards / cost wiring is
//! **sq-py8h.5**. Here the endpoint dedup is over the DISCLOSED endpoint terms,
//! which is exactly the headline case (the endpoints are the disclosed result).
//!
//! ## Randomness (sq-yyro, issue #6117)
//!
//! The chain / union stage draws every sharing through the
//! [`crate::randomness::DistributedRandomness`] seam: each hop equality via the
//! [`crate::compare`] `*_through` chain and each AND / OR via
//! [`crate::shamir::degree_reduce_through`]. The single-dealer simulation's chain
//! / union draws are unchanged from the pre-seam code (a seeded prefix pin test);
//! a source that cannot share a holder's own input (PRSS today) makes every path
//! fail closed. The final
//! oblivious output stage draws from the SAME source
//! (`oblivious_join::oblivious_set_output_through`), so one continuous
//! source covers the whole operator — and because that stage always shares the
//! real-row tag and the payload limbs, even the shapes whose chain stage draws
//! nothing (`k = 1` over distinct endpoint pairs, `p?` over a single predicate
//! with no self-loop and no repeated endpoint pair) now fail closed under PRSS.

use crate::chain_odometer::ChainOdometer;
use crate::compare::secure_equal_to_bit_through;
use crate::field::Fp;
use crate::oblivious_join::{
    oblivious_set_output_through, partial_result_from_slots, Candidate, MatchBit, ObliviousOutput,
    PayloadWidths,
};
use crate::partial::{MpcError, PartialResult};
use crate::randomness::{require_source_matches, DistributedRandomness};
use crate::shamir::{self, ShamirBackend, Share};
use oxrdf::{Term, Variable};
use std::collections::{BTreeMap, BTreeSet};

/// Reserved subject-end / object-end variable names the operator projects onto.
const SUBJECT_VAR: &str = "__pp_a";
const OBJECT_VAR: &str = "__pp_b";

/// The output schema both hidden-path entry points project onto: the endpoint pair.
fn chain_out_vars() -> Vec<Variable> {
    vec![
        Variable::new_unchecked(SUBJECT_VAR),
        Variable::new_unchecked(OBJECT_VAR),
    ]
}

/// Maximum number of edge-tuples (`|E|^k`) the operator will enumerate before
/// refusing the path with a [`MpcError::Protocol`]. The enumeration is
/// `|E|^k` — exponential in the PUBLIC bound `k` and polynomial-of-degree-`k` in
/// `|E|` — and each tuple costs `k − 1` secure equalities + AND/OR chaining, so an
/// unchecked unroll is a CPU/round denial-of-service on this surface. Because both
/// `|E|` and `k` are PUBLIC, the count is computable in closed form WITHOUT touching
/// any secret key, so we reject an over-cap path BEFORE any crypto runs. A protocol
/// guard, not a tuning knob: a path that exceeds it is abusive for this regime.
///
/// ## What this module's caps do and do not bound
///
/// Every cap HERE bounds a COUNT: the hop count, the enumerated tuple count, the
/// candidate count and the output bound `B`. The WIDTHS are bounded by the output
/// stage's own caps, which this operator applies at its ENTRY: the payload arity
/// ([`crate::oblivious_join::MAX_PAYLOAD_ARITY`]), the per-term serialized byte
/// width ([`crate::oblivious_join::MAX_PAYLOAD_TERM_BYTES`]), the uniform per-slot
/// limb count ([`crate::oblivious_join::MAX_SLOT_LIMBS`]), the compute-party count
/// ([`crate::oblivious_join::MAX_OUTPUT_PARTIES`]) and their aggregate product
/// ([`crate::oblivious_join::MAX_SLOT_WORK`]) — see
/// `crate::oblivious_join::check_output_public_shape`, called before this
/// operator builds a candidate or draws a share.
///
/// What is NOT bounded: whole-operator resource planning — dealer batches,
/// aggregate sharing, session traffic and deadlines — is follow-up work, and no
/// claim is made here that it exists.
pub const MAX_CHAIN_TUPLES: u64 = 1 << 18; // 262_144

/// Maximum hop count the operator will consider — the exactly-`k` chain's `k`, and
/// the bounded form's upper bound (`HiddenBoundedPath::max`). A larger one is an
/// [`MpcError::Protocol`] refusal raised BEFORE any enumeration, counting or draw.
///
/// [`MAX_CHAIN_TUPLES`] alone does not bound the hop count. Two loops grow with the
/// hop count and run *while computing* the tuple count that cap is compared
/// against: `projected_tuple_count` multiplies once per hop, and
/// `check_bounded_params` accumulates `Σ_ℓ (a^ℓ·|E|^ℓ)` with an inner loop per
/// length — quadratic in the upper bound. So hop counts are capped before
/// evaluation, on the hop count itself, which is a PUBLIC plan parameter.
///
/// The value is `log2(`[`MAX_CHAIN_TUPLES`]`)`, which makes this guard agree with
/// the tuple guard instead of adding an independent number: for `|E| >= 2` — the
/// only case where a `k`-hop chain has more than one edge-tuple — `|E|^k >= 2^k`,
/// so [`MAX_CHAIN_TUPLES`] already refuses every `k` above this. What the hop cap
/// adds is the `|E| <= 1` corner, where the tuple count stays `1` (or `0`) for any
/// `k` while the per-chain work — `k − 1` secure equalities, and a `k`-wide
/// odometer allocation — still grows with `k`.
///
/// Hops beyond the number of distinct nodes are redundant for path EXISTENCE, which
/// would permit capping the effective bound at the node count instead of refusing.
/// This operator refuses instead: truncating the bound would silently change the
/// evaluated form (the bounded union runs over `min..=max`, so a `min` above the
/// node count is not a no-op), and a fail-closed refusal on a public parameter is
/// the discipline the module's other guards follow. A protocol guard, not a tuning
/// knob.
pub const MAX_HOP_COUNT: usize = MAX_CHAIN_TUPLES.ilog2() as usize; // 18

/// One node of the secret graph: its PRIVATE join key (an `F_p` element — the
/// holder's injective encoding of the node, exactly the contract
/// [`crate::join::HiddenValueJoin`] documents) plus its term.
///
/// The term is NOT used only for matched endpoints. Every edge's subject and object
/// term is read in the clear by the evaluator: as the grouping key of every tuple
/// that starts or ends at this node, as an input to the uniform payload width, and
/// by the candidate-count projections. What the RESULT discloses is narrower than
/// that — only the matched pairs — so the term of an unmatched endpoint is visible
/// to the grouping evaluator without appearing in any answer. The key is the part
/// that is never reconstructed, for an intermediate node or any other.
///
/// `Debug` is hand-written and redacts BOTH fields: a derived one printed the key
/// from every container that holds a node, and a `Debug` rendering is not one of
/// the clear-text uses above. Both fields stay `pub` for the callers that
/// legitimately read them, including the `Term`-keyed groupings in this module.
#[derive(Clone)]
pub struct HiddenNode {
    /// The private join key. Compared via secure equality; never reconstructed
    /// for an intermediate node.
    pub key: Fp,
    /// The node's term. Read in the clear by the grouping, the width sizing and
    /// the projections; emitted in the RESULT only if this node is a matched chain
    /// endpoint.
    pub term: Term,
}

impl std::fmt::Debug for HiddenNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HiddenNode")
            .field("key", &"<redacted>")
            .field("term", &"<redacted>")
            .finish()
    }
}

/// One directed edge of the secret graph for the path predicate: `(subject,
/// object)`, each a [`HiddenNode`]. The edge set is the union of every holder's
/// disclosed edges for the predicate.
#[derive(Debug, Clone)]
pub struct HiddenEdge {
    /// The edge's subject node.
    pub subject: HiddenNode,
    /// The edge's object node.
    pub object: HiddenNode,
}

/// The federated secret edge relation the chain operator traverses — the union of
/// all holders' `(subject, object)` edges for one path predicate.
///
/// **Exactly what is disclosed.** The keys are private. Disclosed: the edge count
/// `|E|` and the candidate-tuple count `|E|^k` (public input sizes), the matched
/// endpoint TERMS (the result), the public bound `B`, the uniform payload width
/// (the longest encoding over ALL candidates, unmatched included), and — through
/// the run's SCHEDULE, on every successful run — the OR-fold positions and hence
/// the distinct endpoint-pair count `D`: the grouping key is the disclosed endpoint
/// pair of every enumerated tuple, and each repeat costs one extra secure OR.
/// **Unmatched endpoint terms are visible to the grouping evaluator**, which reads
/// every tuple's endpoint pair in the clear, although they appear in no answer. The
/// arithmetic protocol does not explicitly open intermediate keys, hop bits, tuple
/// bits or connected bits. This does not hide node identities or connectivity from
/// the simulator host, or from an observer supplied the clear edge terms and their
/// associations; under the documented consistent encoding
/// ([`crate::term_encode::encode_term`]), those terms determine hop equality and
/// chain connectivity. A `B` at or above
/// [`public_candidate_pair_bound`] is decided without computing `D`, which keeps
/// `D` out of the DECISION and out of the error text — not out of the schedule.
#[derive(Debug, Clone)]
pub struct HiddenEdges {
    edges: Vec<HiddenEdge>,
}

impl HiddenEdges {
    /// Build the edge relation from a list of edges (the federated union).
    pub fn new(edges: Vec<HiddenEdge>) -> Self {
        HiddenEdges { edges }
    }

    /// Every endpoint TERM this relation could put in a payload slot, for the
    /// width pass at an operator's entry. A superset of the terms the candidates
    /// carry (every candidate endpoint is an edge endpoint), and reachable before
    /// anything is drawn, which is what lets the width be fixed before the first
    /// sharing.
    pub(crate) fn payload_terms(&self) -> impl Iterator<Item = &Term> {
        self.edges
            .iter()
            .flat_map(|e| [&e.subject.term, &e.object.term])
    }

    /// Number of edges in the relation (`|E|` — public).
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether the relation is empty.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// `|E|^k` (the edge-tuple count), checked against `u64` overflow so the caller can
/// reject the path with a controlled error instead of risking a huge allocation.
fn projected_tuple_count(edge_count: usize, k: usize) -> Option<u64> {
    let e = edge_count as u64;
    let mut total: u64 = 1;
    for _ in 0..k {
        total = total.checked_mul(e)?;
    }
    Some(total)
}

/// A degree-`t` sharing of the PUBLIC constant `c` on the canonical points
/// `x = 1..=n`: the constant polynomial `f(x) = c` at each party. Reconstructs to
/// `c`, is degree `t` (degree 0 ⊆ degree `t`), and composes with secret sharings in
/// the multiplications below with no dealer/randomness. (Mirrors `compare.rs`'s
/// `const_sharing`, local here to avoid coupling across modules.)
fn const_sharing(n: usize, c: Fp) -> Vec<Share> {
    (1..=n as u64).map(|x| Share { x, y: c }).collect()
}

/// Secret AND of two secret-shared 0/1 bits: `[a ∧ b] = [a]·[b]`. One
/// [`shamir::mul_shares_raw`] (degree `2t`) followed by one
/// [`shamir::degree_reduce_through`] back to a fresh degree-`t` sharing — the BGW
/// multiplication-chain step (sq-dvuc) that lets the `k` hop bits chain, its
/// re-sharing randomness drawn through the [`DistributedRandomness`] seam. (Same
/// construction as `compare.rs`'s `secret_and`; local to keep file-lane discipline.)
fn secret_and(
    source: &mut dyn DistributedRandomness,
    a: &[Share],
    b: &[Share],
) -> Result<Vec<Share>, MpcError> {
    let prod_2t = shamir::mul_shares_raw(a, b)?;
    shamir::degree_reduce_through(source, &prod_2t)
}

/// Secret OR of two secret-shared 0/1 bits via De Morgan: `[a ∨ b] = a + b − a·b`.
/// One secure multiplication (the `a·b` term); the rest is local affine ops. Used
/// to OR-fold the per-tuple chain bits of the chains that reach the same endpoint
/// pair into one connected-bit (design §2.5).
fn secret_or(
    source: &mut dyn DistributedRandomness,
    a: &[Share],
    b: &[Share],
) -> Result<Vec<Share>, MpcError> {
    let and = secret_and(source, a, b)?;
    let sum = shamir::add_shares(a, b)?;
    shamir::sub_shares(&sum, &and)
}

/// The chain bit of ONE edge-tuple `(e_1, …, e_k)`: the secret-shared AND over the
/// `k − 1` internal hop equalities `[e_i.object_key == e_{i+1}.subject_key]`, each a
/// [`crate::compare::secure_equal_to_bit`] (secret-shared, never opened; drawn
/// through the seam). For `k = 1` there is no internal hop, so the bit is the
/// constant shared `1` (every edge is a length-1 chain). The intermediate node KEYS
/// are passed to the secure equality but never reconstructed.
fn tuple_chain_bit(
    backend: &ShamirBackend,
    source: &mut dyn DistributedRandomness,
    tuple: &[&HiddenEdge],
) -> Result<Vec<Share>, MpcError> {
    let n = backend.parties();
    debug_assert!(!tuple.is_empty());
    // k = 1: trivially connected (no internal hop) → constant shared 1.
    let mut bit = const_sharing(n, Fp::one());
    for w in tuple.windows(2) {
        // The internal hop join: e_i.object must equal e_{i+1}.subject. Both are
        // intermediate node key sharings the protocol never opens, and the equality
        // verdict stays a secret-shared bit — the arithmetic reconstructs neither.
        // The clear edge terms the caller supplies still determine this verdict
        // under the consistent encoding (`crate::term_encode::encode_term`); see the
        // module's disclosure list.
        let hop = secure_equal_to_bit_through(source, w[0].object.key, w[1].subject.key)?;
        bit = secret_and(source, &bit, &hop)?;
    }
    Ok(bit)
}

/// A disclosed endpoint pair key for grouping tuples (set semantics). Built from the
/// DISCLOSED endpoint terms — never any secret intermediate key.
fn endpoint_key(a: &Term, b: &Term) -> String {
    format!("{a:?}\u{0}{b:?}")
}

/// Build one [`Candidate`] per DISTINCT disclosed endpoint pair, OR-folding the
/// chain bits of every edge-tuple that reaches that pair into one secret-shared
/// connected-bit (design §2.5 — set semantics; the realized length is not
/// separately opened or encoded in the result).
///
/// Iterates the `|E|^k` edge-tuples lexicographically, computes each tuple's chain
/// bit ([`tuple_chain_bit`]), and accumulates it into the connected-bit for the
/// tuple's endpoint pair `(e_1.subject, e_k.object)`. The grouping key is the
/// disclosed endpoint pair; the connected-bit and the per-tuple chain bits are all
/// secret-shared and never opened here.
fn build_candidates(
    backend: &ShamirBackend,
    source: &mut dyn DistributedRandomness,
    edges: &HiddenEdges,
    k: usize,
) -> Result<Vec<Candidate>, MpcError> {
    // Accumulator: disclosed endpoint key → (endpoint terms, OR-folded connected bit).
    let mut by_pair: BTreeMap<String, (Term, Term, Vec<Share>)> = BTreeMap::new();

    // Lexicographic enumeration of the |E|^k edge-tuples via a base-|E| odometer.
    let m = edges.len();
    if m == 0 {
        return Ok(Vec::new()); // no edges → no chains
    }
    let mut idx = vec![0usize; k];
    loop {
        // Materialise the current tuple as edge references.
        let tuple: Vec<&HiddenEdge> = idx.iter().map(|&i| &edges.edges[i]).collect();
        let chain_bit = tuple_chain_bit(backend, source, &tuple)?;
        let a_term = tuple[0].subject.term.clone();
        let b_term = tuple[k - 1].object.term.clone();
        let key = endpoint_key(&a_term, &b_term);
        match by_pair.get_mut(&key) {
            Some((_, _, acc)) => {
                // OR this tuple's chain bit into the running connected-bit.
                let folded = secret_or(source, acc, &chain_bit)?;
                *acc = folded;
            }
            None => {
                by_pair.insert(key, (a_term, b_term, chain_bit));
            }
        }

        // Advance the odometer (least-significant position is idx[k-1]).
        let mut pos = k;
        loop {
            if pos == 0 {
                // Wrapped all positions → enumeration complete.
                return Ok(by_pair
                    .into_values()
                    .map(|(a, b, bit)| Candidate {
                        payload: vec![Some(a), Some(b)],
                        matched: MatchBit::SecretShared(bit),
                    })
                    .collect());
            }
            pos -= 1;
            idx[pos] += 1;
            if idx[pos] < m {
                break;
            }
            idx[pos] = 0;
        }
    }
}

/// An upper bound on the candidate count of [`eval_exact_k_chain_hidden`],
/// computed from the PUBLIC input sizes alone — the edge count and `k`, both of
/// which this module already discloses. Nothing here looks at an endpoint term.
///
/// This is the value to provision the public padded bound `B` against. A `B` at or
/// above it is decided on public sizes alone, so the decision and its error text
/// never consult the edge data; a tighter `B` is still accepted when the distinct
/// endpoint-pair count `D` fits, at the cost of disclosing that comparison (see
/// `eval_exact_k_chain_hidden`). Neither choice hides `D` itself: the grouping
/// reads every tuple's endpoint pair in the clear and each repeat costs one extra
/// multiplication, on either branch.
///
/// `build_candidates` groups the `|E|^k` edge-tuples by their first subject and
/// last object, so:
///
/// - `k == 1` — one position, so at most one key per edge: `|E|`.
/// - `k >= 2` — the two positions are independent odometer digits, so at most the
///   product of the distinct subjects with the distinct objects, each at most
///   `|E|`: `|E|^2`. The tuple count `|E|^k` bounds it too, so the answer is the
///   smaller of the two.
///
/// Returns `MpcError::Protocol` if `|E|^2` overflows `usize`.
pub fn public_candidate_pair_bound(edge_count: usize, k: usize) -> Result<usize, MpcError> {
    if edge_count == 0 || k == 0 {
        return Ok(0);
    }
    if k == 1 {
        return Ok(edge_count);
    }
    let squared = edge_count.checked_mul(edge_count).ok_or_else(|| {
        MpcError::Protocol(format!(
            "hidden exactly-k chain: the public endpoint-pair bound |E|^2 overflows usize for \
             |E|={}",
            edge_count
        ))
    })?;
    Ok(
        match projected_tuple_count(edge_count, k).map(usize::try_from) {
            Some(Ok(tuples)) => squared.min(tuples),
            // `|E|^k` either overflowed `u64` or does not fit a `usize` on this
            // target; it cannot be the smaller of the two bounds.
            _ => squared,
        },
    )
}

/// An upper bound on the candidate count of [`eval_bounded_path_hidden`], from the
/// PUBLIC input sizes alone — the edge count and the form's public `min`/`max`.
///
/// Same contract as [`public_candidate_pair_bound`], for the bounded union. The
/// builder keys every enumerated tuple's endpoint pair plus, when `min == 0`, the
/// reflexive diagonal over the DECLARED public node domain, so the arms are
/// bounded publicly by
///
/// - `min == 0` — one diagonal pair per declared node, so exactly
///   `node_domain_len` pairs (the declared domain's SIZE is a public plan input).
///   Pass `0` for an edge relation whose declared domain is EMPTY, and also for
///   one that declares no domain at all — the two are indistinguishable here. The
///   second cannot serve a `min == 0` form: the evaluation refuses when it reaches
///   the diagonal, whatever this bound says. The first serves it with an empty
///   diagonal, which is the right answer for an empty active graph;
/// - length `1` in range — at most `|E|` pairs;
/// - any length `>= 2` in range — at most `|E|^2` pairs.
///
/// The answer is their SUM, which over-counts the pairs the arms share. That is
/// deliberate: the bound must not depend on the edge data, and over-counting only
/// raises the `B` at which the decision becomes data-independent. For inputs
/// `check_bounded_params` accepts, that sum can exceed
/// [`crate::oblivious_join::MAX_OUTPUT_BOUND`] — a `{0,2}` form only needs an
/// `|E|` whose square passes `MAX_CHAIN_TUPLES` but whose `|E|^2` term alone
/// passes `MAX_OUTPUT_BOUND`, and the diagonal arm adds the declared domain on
/// top — and then no admissible `B` reaches it, so the caller
/// has only the data-dependent branch. Neither branch hides the distinct
/// endpoint-pair count: the grouping reads every tuple's endpoint pair in the clear
/// and each repeat costs one extra multiplication, whichever branch decided `B`.
///
/// Returns `MpcError::Protocol` if the sum overflows `usize`.
pub fn public_bounded_candidate_pair_bound(
    edge_count: usize,
    node_domain_len: usize,
    form: &HiddenBoundedPath,
) -> Result<usize, MpcError> {
    let overflow = || {
        MpcError::Protocol(format!(
            "hidden bounded path: the public endpoint-pair bound overflows usize for |E|={}",
            edge_count
        ))
    };
    let mut total = 0usize;
    // The diagonal arm is sized by the DECLARED domain, which is independent of
    // `|E|`: a graph whose other predicates name many nodes has a domain larger
    // than `2*|E|`, and one with no declared domain never reaches here with
    // `min == 0`. So it is added even when `|E| == 0` — the builder emits the
    // diagonal for every declared node whether or not any edge exists.
    if form.min == 0 {
        total = total.checked_add(node_domain_len).ok_or_else(overflow)?;
    }
    if edge_count == 0 {
        return Ok(total);
    }
    if form.min <= 1 && form.max >= 1 {
        total = total.checked_add(edge_count).ok_or_else(overflow)?;
    }
    if form.max >= 2 {
        total = total
            .checked_add(edge_count.checked_mul(edge_count).ok_or_else(overflow)?)
            .ok_or_else(overflow)?;
    }
    Ok(total)
}

/// The number of candidates [`build_candidates`] will produce, computed from the
/// DISCLOSED endpoint terms alone in `O(|E| log |E|)` — before a single sharing is
/// drawn.
///
/// `build_candidates` groups the `|E|^k` edge-tuples by
/// `endpoint_key(tuple[0].subject, tuple[k-1].object)`, and the odometer runs
/// positions `0` and `k-1` independently, so the key set is determined by the edge
/// endpoints and `k` alone:
///
/// - `k == 1` — one position, so the keys are the distinct `(subject, object)`
///   pairs of the edges themselves.
/// - `k >= 2` — positions `0` and `k-1` are different odometer digits and every
///   combination is enumerated, so the keys are the full product of the distinct
///   subject terms with the distinct object terms.
///
/// Counting is done over the same `{:?}` representation the grouping key is built
/// from, so the result is the builder's own candidate count — EXCEPT in the one
/// degenerate case where a term's representation contains the key's NUL separator,
/// which would make two distinct pairs share a key; the count is then an upper
/// bound. Either way it never UNDER-counts, so refusing on it is fail-closed, and
/// the exact post-build check stays.
fn projected_candidate_count(edges: &HiddenEdges, k: usize) -> Result<usize, MpcError> {
    if edges.is_empty() || k == 0 {
        return Ok(0);
    }
    if k == 1 {
        let keys: BTreeSet<String> = edges
            .edges
            .iter()
            .map(|e| endpoint_key(&e.subject.term, &e.object.term))
            .collect();
        return Ok(keys.len());
    }
    let subjects: BTreeSet<String> = edges
        .edges
        .iter()
        .map(|e| format!("{:?}", e.subject.term))
        .collect();
    let objects: BTreeSet<String> = edges
        .edges
        .iter()
        .map(|e| format!("{:?}", e.object.term))
        .collect();
    subjects.len().checked_mul(objects.len()).ok_or_else(|| {
        MpcError::Protocol(format!(
            "hidden exactly-k chain: the endpoint-pair count {} x {} overflows usize — \
             refusing before any draw",
            subjects.len(),
            objects.len()
        ))
    })
}

/// The number of candidates [`build_bounded_candidates`] will produce, computed
/// from the DISCLOSED endpoint terms alone and before any draw.
///
/// The builder folds every enumerated tuple's endpoint pair in regardless of
/// whether the tuple's predicates matched the chain (a mismatch contributes a
/// constant `0` bit, but the PAIR is still keyed), so the key set does not depend
/// on the alternation at all. It is the UNION of:
///
/// - the reflexive diagonal `(x, x)` over the DECLARED node domain, when
///   `min == 0`;
/// - the distinct `(subject, object)` edge pairs, when length `1` is in range;
/// - the full product of distinct subjects with distinct objects, when any length
///   `>= 2` is in range.
///
/// The union is materialised rather than estimated, because a sum of the three
/// arms double-counts the pairs they share and would refuse a `bound` the operator
/// should accept. It costs at most the keys the builder's own accumulator will
/// hold: the product arm is `|A| x |B| <= |E|^2`, and `check_bounded_params` —
/// which runs first — has already capped `Σ_ℓ a^ℓ·|E|^ℓ` at
/// [`MAX_CHAIN_TUPLES`], so for any form that reaches the product arm `|E|^2` is
/// within that cap. What it avoids is the `Σ_ℓ |E|^ℓ` secure equalities the build
/// would have drawn before the same refusal.
fn projected_bounded_candidate_count(
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
) -> Result<usize, MpcError> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    if form.min == 0 {
        for node in edges.node_domain()? {
            keys.insert(endpoint_key(node, node));
        }
    }
    if !edges.is_empty() {
        let lo = form.min.max(1);
        if lo <= form.max {
            if lo == 1 {
                for e in &edges.edges {
                    keys.insert(endpoint_key(&e.subject.term, &e.object.term));
                }
            }
            if form.max >= 2 {
                // `Term` is not `Ord`, so dedup on the same `{:?}` representation
                // `endpoint_key` is built from.
                let mut subjects: BTreeMap<String, &Term> = BTreeMap::new();
                let mut objects: BTreeMap<String, &Term> = BTreeMap::new();
                for e in &edges.edges {
                    subjects
                        .entry(format!("{:?}", e.subject.term))
                        .or_insert(&e.subject.term);
                    objects
                        .entry(format!("{:?}", e.object.term))
                        .or_insert(&e.object.term);
                }
                for a in subjects.values() {
                    for b in objects.values() {
                        keys.insert(endpoint_key(a, b));
                    }
                }
            }
        }
    }
    Ok(keys.len())
}

/// Fail-closed parameter checks shared by both entry points. `k >= 1`, `k` within
/// [`MAX_HOP_COUNT`], `n >= 2t+1` (the AND/OR chain's degree reduction needs it),
/// and the projected `|E|^k` tuple count within [`MAX_CHAIN_TUPLES`] — all computed
/// from PUBLIC parameters, before any crypto runs.
///
/// The hop cap is checked BEFORE the tuple count, because `projected_tuple_count`
/// itself loops once per hop: hop counts are capped before evaluation.
fn check_params(backend: &ShamirBackend, edges: &HiddenEdges, k: usize) -> Result<(), MpcError> {
    if k > MAX_HOP_COUNT {
        return Err(MpcError::Protocol(format!(
            "hidden exactly-k chain with k={} exceeds the MAX_HOP_COUNT cap of {} — refusing \
             (each hop adds a secure equality to every enumerated chain, so the hop count is \
             bounded before the enumeration is sized)",
            k, MAX_HOP_COUNT
        )));
    }
    if k == 0 {
        return Err(MpcError::Protocol(
            "hidden exactly-k chain: k must be >= 1 (the reflexive length-0 arm is the {0,k} \
             increment sq-py8h.3, not the exactly-k chain operator)"
                .into(),
        ));
    }
    let t = backend.threshold();
    if backend.parties() < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "hidden exactly-k chain needs n >= 2t+1 for the AND/OR chain's degree reduction \
             (n={}, t={t})",
            backend.parties()
        )));
    }
    match projected_tuple_count(edges.len(), k) {
        Some(count) if count <= MAX_CHAIN_TUPLES => Ok(()),
        other => Err(MpcError::Protocol(format!(
            "hidden exactly-k chain over |E|={} edges with k={k} would enumerate {} edge-tuples, \
             exceeding the MAX_CHAIN_TUPLES cap of {} — refusing (CPU/round denial-of-service)",
            edges.len(),
            other
                .map(|c| c.to_string())
                .unwrap_or_else(|| "an amount that overflows u64".to_string()),
            MAX_CHAIN_TUPLES,
        ))),
    }
}

/// Evaluate the HIDDEN-intermediate exactly-`k` chain and return the disclosed
/// endpoint pairs as a [`PartialResult`] with `vars == [?__pp_a, ?__pp_b]` (real
/// rows only, dummies filtered, canonical order). The intermediate node keys are
/// never reconstructed; each per-pair connected-bit stays secret-shared until the
/// oblivious output's `B`-slot reveal.
///
/// `bound` is the public padded bound `B`; it must cover the distinct-endpoint-pair
/// candidate count (fail-closed BEFORE any draw — never truncates a match).
///
/// **How `B` is decided, and what the decision discloses.** `B` is first compared
/// against [`public_candidate_pair_bound`], an upper bound from the PUBLIC
/// input sizes. A `B` at or above it is accepted on those sizes alone: the distinct
/// endpoint-pair count `D` is not even computed, so the DECISION and its error text
/// rest on public sizes only. A `B` below it is accepted exactly when `D <= B`, so
/// success-or-refusal discloses that comparison; and because a refusal draws
/// nothing, repeated calls with different `B` recover `D` exactly by binary search.
/// The search only pins `D` up to
/// [`crate::oblivious_join::MAX_OUTPUT_BOUND`], since a larger `B` is refused by the
/// cap before either branch; past that the caller learns only that `D` is larger.
/// Either way the refusal message names only `B` and the public bound — and NEITHER
/// branch hides `D` from a successful run. The grouping keys every enumerated tuple
/// by its endpoint pair, read in the clear: in the model as built one process
/// simulates every party, so the grouping evaluator sees the ordered candidate pair
/// sequence. In a distributed implementation that supplies every evaluating party
/// with the ordered plaintext endpoint terms, or with this construction's
/// data-dependent accumulator wiring, every such party can determine the fold
/// positions. This crate implements a single-process simulation and does not
/// establish a per-party input or wiring-distribution boundary.
/// The `T = |E|^k` tuples cost `T − D` ORs, and the output
/// pads from `D` to `B`, so the multiplication count carries `D` regardless. Hiding
/// `D` would need oblivious grouping with a uniform schedule, which is not built;
/// see the module docs for the transport note and the limits of this account. A
/// public-half refusal happens before any candidate exists, and a tight-bound
/// refusal runs the data-dependent projection but no build — neither leaves a draw
/// schedule.
///
/// One fresh dealer covers the chain stage AND the output
/// stage: this is `eval_exact_k_chain_hidden_slots_through` on that one dealer,
/// narrowed to the disclosed rows — a single code path, so nothing here can mint a
/// second dealer. See the module docs for the full construction and security tier.
pub fn eval_exact_k_chain_hidden(
    backend: &ShamirBackend,
    edges: &HiddenEdges,
    k: usize,
    bound: usize,
) -> Result<PartialResult, MpcError> {
    let (slots, _cost) =
        eval_exact_k_chain_hidden_slots_through(&mut backend.dealer(), backend, edges, k, bound)?;
    Ok(partial_result_from_slots(slots, chain_out_vars()))
}

/// Like [`eval_exact_k_chain_hidden`] but returns the raw oblivious output (the `B`
/// shuffled [`crate::oblivious_join::OutputSlot`]s + modelled cost) instead of the
/// dummy-filtered [`PartialResult`]. This is the transcript-level view: it
/// witnesses that the revealed slot count is exactly the public `B` (L1 bounded to
/// `B`, not the true match count). Used by the obliviousness tests.
pub fn eval_exact_k_chain_hidden_slots(
    backend: &ShamirBackend,
    edges: &HiddenEdges,
    k: usize,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    eval_exact_k_chain_hidden_slots_through(&mut backend.dealer(), backend, edges, k, bound)
}

/// [`eval_exact_k_chain_hidden_slots`] with the chain stage's randomness drawn
/// through the [`DistributedRandomness`] seam (sq-yyro, issue #6117): every hop
/// equality via the [`crate::compare`] `*_through` chain and every AND / OR via
/// [`shamir::degree_reduce_through`]. Under the single-dealer simulation the
/// REVEALED multiset and the modelled cost are unchanged from the pre-seam code,
/// and the chain prefix of the draw transcript is pinned unchanged; the revealed
/// slot ORDER moved, since the shuffle now samples a different point of one
/// continuous keystream rather than a second dealer's fresh one. A source
/// whose `(n, t)` differs from `backend`'s is refused
/// ([`require_source_matches`]); a source that cannot share a holder's own input
/// (PRSS today) makes the evaluation fail closed with that source's error. The
/// oblivious output stage draws from the SAME `source`
/// ([`oblivious_set_output_through`]) — one continuous source for the whole
/// operator, where it previously minted a second dealer from `backend` and
/// restarted the keystream.
pub(crate) fn eval_exact_k_chain_hidden_slots_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    edges: &HiddenEdges,
    k: usize,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    require_source_matches(source, backend)?;
    check_params(backend, edges, k)?;
    // BOTH halves of the bound contract are settled before any draw, and the PUBLIC
    // half is tried first: a `B` at or above `public_candidate_pair_bound` is
    // accepted on the edge count and `k` alone, and the data-dependent projection
    // below is never run. Only a tighter `B` needs the real count, which is then
    // computed from the disclosed endpoint terms in O(|E| log |E|). This keeps the
    // count out of the DECISION and the error text; the build below still allocates
    // and multiplies per distinct pair, so the count remains visible in the
    // schedule. Leaving the
    // coverage half to the output stage meant `build_candidates` had already drawn
    // one equality per hop of every one of the `|E|^k` tuples before a `B` too small
    // to hold the answer was refused — up to billions of draws for a refusal that
    // was decidable without any. Both checks sit after `check_params` so the
    // operator's own parameter refusals keep precedence; neither of those draws.
    crate::oblivious_join::check_bound_public_first(
        "hidden exactly-k chain",
        bound,
        public_candidate_pair_bound(edges.len(), k)?,
        || projected_candidate_count(edges, k),
    )?;
    // Payload arity is 2 (the endpoint pair). The stage's public-shape caps run
    // HERE, before any candidate is built or any share drawn; they read only public
    // numbers, and the stage re-checks them.
    let widths = PayloadWidths::from(2);
    // Resolve the widths over the EDGE endpoints — a superset of the terms the
    // candidates will carry — in one streaming pass that retains nothing. In the
    // default (derived) mode this is what fixes the width before any draw; in the
    // declared mode it is what refuses an over-width term before any draw.
    let resolved = widths.resolve(edges.payload_terms())?;
    crate::oblivious_join::check_output_public_shape(
        &resolved,
        bound,
        backend.parties(),
        backend.threshold(),
    )?;
    let candidates = build_candidates(backend, source, edges, k)?;
    // Route through the slot-returning path. The stage re-checks the bound against
    // the EXACT candidate count, which is what catches the degenerate case where the
    // projection above could only bound it.
    oblivious_set_output_through(source, backend, &candidates, widths, bound)
}

/// Test-only instrumentation: the candidates
/// `eval_exact_k_chain_hidden_slots_through` builds, together with the
/// `(is_real, decoded_row)` of every slot the REAL output stage opens for them.
/// Same candidate construction and same output stage as the public path — it just
/// keeps the dummy slots' decoded payload rows, which [`ObliviousOutput`] cannot
/// express.
#[cfg(test)]
#[allow(clippy::type_complexity)]
pub(crate) fn exact_k_chain_candidates_and_opened_rows(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    edges: &HiddenEdges,
    k: usize,
    bound: usize,
) -> Result<(Vec<Candidate>, Vec<(bool, Vec<Option<Term>>)>), MpcError> {
    require_source_matches(source, backend)?;
    check_params(backend, edges, k)?;
    crate::oblivious_join::check_bound_public_first(
        "hidden exactly-k chain",
        bound,
        public_candidate_pair_bound(edges.len(), k)?,
        || projected_candidate_count(edges, k),
    )?;
    let candidates = build_candidates(backend, source, edges, k)?;
    let slots =
        crate::oblivious_join::opened_slot_rows_through(source, backend, &candidates, 2, bound)?;
    Ok((candidates, slots))
}

// =====================================================================
// sq-py8h.3 — bounded repetition `{1,k}` / `{0,k}` / `p?` + alternation
// over the SAME hidden edge set, as a union-of-fixed-chains with an
// OR-fold cross-length dedup (the realized length is not separately opened
// or encoded in the result).
// =====================================================================

/// One labelled directed edge of the secret graph: `(subject, object)` nodes plus a
/// DISCLOSED predicate label. The predicate is public (it is named in the public
/// query plan, design §1.2/§4.1) and is used ONLY to select which alternation branch
/// an edge may serve at a hop position; the node *keys* stay private exactly as in
/// [`HiddenEdge`].
#[derive(Debug, Clone)]
pub struct PredicatedEdge {
    /// The (public) predicate IRI this edge belongs to.
    pub predicate: String,
    /// The edge's subject node (private key + disclosed term).
    pub subject: HiddenNode,
    /// The edge's object node (private key + disclosed term).
    pub object: HiddenNode,
}

/// The federated secret edge relation for a bounded path whose hops may range over an
/// ALTERNATION of predicates `p_1 | … | p_a`. It is the union of every holder's
/// `(predicate, subject, object)` edges. Mirrors [`HiddenEdges`] but tags each
/// edge with its (public) predicate so a hop can be restricted to one alternation
/// branch.
///
/// **What this operator discloses is the module's disclosure contract** — see the
/// [module docs][crate::hidden_path] and [`HiddenEdges`]. It is wider than the
/// candidate-tuple count and the matched endpoint terms: the grouping reads EVERY
/// enumerated tuple's endpoint pair in the clear (unmatched ones included), the
/// schedule carries the distinct endpoint-pair count, the uniform payload width is
/// disclosed, and for `min == 0` the declared node domain's terms and size are
/// public plan inputs.
#[derive(Clone)]
pub struct PredicatedEdges {
    edges: Vec<PredicatedEdge>,
    /// The caller-declared PUBLIC node domain of the active graph, if any.
    /// Deduped and canonically ordered; any `Term`, literals included. `None` =
    /// not declared, and a form with `min == 0` is then refused — see
    /// [`PredicatedEdges::with_node_domain`].
    node_domain: Option<Vec<Term>>,
}

/// `Debug` is hand-written, like [`HiddenNode`]'s, and for the same reason: a
/// derived one would print every term this relation owns from every container
/// above it. [`PredicatedEdge`] already redacts its own nodes, but
/// `node_domain` holds endpoint terms directly — including the terms of
/// intermediate nodes, which `HiddenNode`'s `Debug` redacts — so a derived
/// `Debug` on this type would have printed exactly what that redaction withholds.
///
/// What prints instead: the edge COUNT and the declared domain's SIZE (or that
/// none is declared). Both are already public — `|E|` is a public input size and
/// the domain's size enters [`public_bounded_candidate_pair_bound`] — so neither
/// adds anything a `Debug` rendering should not carry. The edges themselves are
/// not printed at all, only counted — [`PredicatedEdge`]'s own redacting `Debug`
/// still covers them wherever one is printed on its own.
impl std::fmt::Debug for PredicatedEdges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PredicatedEdges")
            .field("edges", &self.edges.len())
            .field(
                "declared_node_domain",
                &match &self.node_domain {
                    Some(d) => format!("<redacted; {} term(s)>", d.len()),
                    None => "<none declared>".to_string(),
                },
            )
            .finish()
    }
}

impl PredicatedEdges {
    /// Build the relation from the federated union of labelled edges.
    ///
    /// No node domain is declared, so a form with `min == 0` (a zero-length arm)
    /// is refused; see [`PredicatedEdges::with_node_domain`].
    pub fn new(edges: Vec<PredicatedEdge>) -> Self {
        PredicatedEdges {
            edges,
            node_domain: None,
        }
    }

    /// Declare the active graph's PUBLIC node domain — every term occurring as a
    /// subject or object of any triple the federation holds — so a zero-length arm
    /// (`{0,k}` / `p?`) can be evaluated.
    ///
    /// ## Why this has to come from the caller
    ///
    /// SPARQL 1.1 §18.4 `ZeroLengthPath` binds the zero-length match to **every
    /// distinct subject or object term of the active graph** — object-only nodes
    /// and **literals** included, predicate-only terms excluded, exactly as the
    /// clear-text engine's `graph_nodes` computes it. The domain is accordingly a
    /// set of [`Term`]s, independent of the regime applied to the edges
    /// themselves: the diagonal is built from the DISCLOSED term, with a constant
    /// shared `1` chain bit and no secure equality, so a non-IRI term is never a
    /// key anything is compared on. A [`crate::partial::PartialResult`] row is
    /// `Vec<Option<Term>>`, so a literal endpoint needs nothing special to carry.
    /// The edges in this relation are only those the
    /// holders contributed for the path's own alternation predicates, so their
    /// endpoint terms are a SUBSET of that domain: a node occurring only in a
    /// triple of another predicate is absent. Drawing the reflexive diagonal from
    /// the edge endpoints alone therefore dropped those `(x, x)` pairs and
    /// answered a `{0,k}` / `p?` form with fewer pairs than the clear-text
    /// engine — a wrong answer, silently.
    ///
    /// Having the operator reach for the wider term set instead is not available
    /// to it and would not be its call: the holders decide which triples they
    /// contribute. So the domain is a declared, PUBLIC plan input, and without it
    /// [`eval_bounded_path_hidden`] refuses a `min == 0` form fail-closed, BEFORE
    /// any sharing is drawn. A form with `min >= 1` never reads the domain.
    ///
    /// Note what declaring it makes public: the diagonal's terms appear in the
    /// opened output exactly as the matched endpoint terms already do, and the
    /// domain's SIZE enters the public candidate bound
    /// ([`public_bounded_candidate_pair_bound`]). Both are properties of the
    /// public plan input the caller chose to declare, not of the secret node keys.
    ///
    /// ## The precondition — what the caller is trusted for
    ///
    /// The caller is trusted to supply **exactly** the selected active graph's
    /// subject/object domain. The only thing this method checks is that the
    /// declaration COVERS every endpoint term of every edge in this relation
    /// (those are terms of the active graph by construction, so a declaration
    /// missing one is inconsistent with the edges it accompanies, and is refused
    /// with [`MpcError::Protocol`]). **Neither completeness nor the absence of
    /// extra nodes is verified:** an omission loses `(x, x)` answers the engine
    /// returns, and an extra node invents an `(x, x)` answer the engine does not.
    /// Both are wrong answers this method cannot detect.
    ///
    /// ### Reproducing the engine's self-match for an ABSENT bound endpoint
    ///
    /// The engine matches a **bound** endpoint against itself even when that
    /// constant occurs in no triple (`sparq-engine`'s `exec/bgp.rs` skips the
    /// node-membership check for a concrete term). A caller can reproduce that by
    /// including the constant in the declared domain — no API change is needed.
    ///
    /// It is a deliberate per-query WIDENING of the domain, not an exception to
    /// the "exactly the domain" precondition above: the widened set is no longer
    /// the active graph's domain, so it is declared for THAT query and the
    /// consequence is accepted knowingly. The scope in which it is correct is a
    /// SINGLE repetition whose bound endpoint is that constant — which is the
    /// whole of THIS regime, since [`HiddenBoundedPath`] is one repetition and
    /// this module has no sequence form at all.
    ///
    /// The caveat therefore belongs to the disclosed-key module, whose
    /// [`crate::bounded_path::PathForm::Sequence`] can chain nullable arms: when
    /// such a sequence is nullable THROUGHOUT and the bound endpoint is the
    /// widened constant, its midpoints range over the declared domain and the
    /// operator answers `(absent, absent)` where the engine answers nothing — at
    /// most that one extra pair, and not at all once any arm has `min >= 1`. A
    /// `{0,k}` range is one repetition, not a chain of `p?` arms, so it is not a
    /// case of it. See
    /// [`crate::bounded_path::DisclosedEdges::with_node_domain`].
    ///
    /// Duplicates are collapsed and the stored order is canonical, so the
    /// diagonal does not depend on the order the caller listed its nodes in.
    pub fn with_node_domain(
        mut self,
        nodes: impl IntoIterator<Item = Term>,
    ) -> Result<Self, MpcError> {
        let mut declared: BTreeMap<String, Term> = BTreeMap::new();
        for n in nodes {
            declared.entry(format!("{:?}", n)).or_insert(n);
        }
        // Count the uncovered endpoint terms; the message names the count only.
        let mut missing: BTreeSet<String> = BTreeSet::new();
        for e in &self.edges {
            for term in [&e.subject.term, &e.object.term] {
                let key = format!("{:?}", term);
                if !declared.contains_key(&key) {
                    missing.insert(key);
                }
            }
        }
        if !missing.is_empty() {
            // VALUE- AND COUNT-FREE. An earlier version named how many endpoints
            // were uncovered and how many terms were declared; both are statistics
            // over the edge relation, and a caller could probe one candidate term
            // at a time and read the count as a membership answer. The text now
            // carries neither.
            return Err(MpcError::Protocol(
                "the declared node domain does not cover the edge endpoints — every edge \
                 endpoint is a term of the active graph, so a zero-length arm over this \
                 domain would under-answer; declare the full subject/object domain of the \
                 active graph"
                    .into(),
            ));
        }
        self.node_domain = Some(declared.into_values().collect());
        Ok(self)
    }

    /// Every term this edge set could put in a payload slot: both endpoints of
    /// every edge, and every node of the declared domain, which a zero-length arm
    /// emits as a `(x, x)` diagonal row. A superset of what the candidates carry,
    /// reachable before anything is drawn.
    pub(crate) fn payload_terms(&self) -> impl Iterator<Item = &Term> {
        self.edges
            .iter()
            .flat_map(|e| [&e.subject.term, &e.object.term])
            .chain(self.node_domain.iter().flatten())
    }

    /// Size of the declared PUBLIC node domain, or `0` when none was declared.
    /// This is the public input size the `min == 0` arm of
    /// [`public_bounded_candidate_pair_bound`] is computed from. `0` is returned
    /// both for an explicitly EMPTY declared domain, whose diagonal is genuinely
    /// empty, and for an undeclared one, which cannot serve a `min == 0` form at
    /// all — that case is refused when the diagonal is reached, so the bound it
    /// would produce cannot let a run through; see `zero_length_domain_err`.
    fn declared_domain_len(&self) -> usize {
        self.node_domain.as_ref().map_or(0, Vec::len)
    }

    /// Number of edges (`|E|` — public).
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether the relation is empty.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// The DECLARED public node domain over which the reflexive (length-0)
    /// identity pairs `(x, x)` are added for `{0,k}` / `p?` (design §2.3; the
    /// analogue of the clear-text engine's `graph_nodes`, which ranges over the
    /// WHOLE active graph). Disclosed terms only; no key is touched. Each distinct
    /// node appears once, in canonical (debug-string) order so the output is
    /// deterministic.
    ///
    /// Fail-closed when no domain was declared: the edge relation's own endpoint
    /// terms are only a subset of the domain, so there is nothing sound to fall
    /// back on. See `PredicatedEdges::with_node_domain`.
    fn node_domain(&self) -> Result<&[Term], MpcError> {
        self.node_domain
            .as_deref()
            .ok_or_else(zero_length_domain_err)
    }
}

/// The fail-closed refusal of a zero-length arm (`min == 0`) on an edge relation
/// that carries no declared public node domain. The single source of that
/// refusal: every consumer of the diagonal — `build_bounded_candidates` and the
/// `projected_bounded_candidate_count` projection — reaches it through
/// `PredicatedEdges::node_domain`, and the diagonal is the first thing the build
/// does, so the refusal lands before any sharing is drawn. Value-free: it names
/// no term and no count.
fn zero_length_domain_err() -> MpcError {
    MpcError::Protocol(
        "hidden bounded path with min == 0 (the zero-length arm of {0,k} / p?) needs the \
         active graph's PUBLIC node domain, which the edge relation does not carry: SPARQL \
         1.1 §18.4 binds the zero-length match to every distinct SUBJECT or OBJECT term of \
         the active graph, including terms occurring only in triples of predicates this path \
         does not traverse. Refusing \
         rather than drawing the reflexive diagonal from the edge endpoints alone, which \
         would silently under-answer. Declare the domain with \
         PredicatedEdges::with_node_domain, or use a form with min >= 1."
            .into(),
    )
}

/// A bounded property-path form the HIDDEN-regime operator understands: a repetition
/// `(step){min,max}` of a single step, where the step is an ALTERNATION of one or more
/// (public) predicate IRIs `p_1 | … | p_a` (a bare predicate is `a == 1`).
///
/// - `{k,k}` is exactly-`k`.
/// - `{1,k}` is the bounded `+`.
/// - `{0,k}` is the bounded `*` (adds the length-0 reflexive diagonal, design §2.3).
/// - `{0,1}` is `p?` ([`HiddenBoundedPath::optional`]).
///
/// The bounds are PUBLIC plan parameters; an OPEN upper bound (`+`/`*`/`{m,}`) is NOT
/// representable here by construction (`max` is a concrete `usize`) — the
/// fail-closed refusal of a hidden unbounded path is a planner concern (sq-py8h.5),
/// not this operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenBoundedPath {
    /// The (public) alternation predicates a single hop may traverse. Non-empty.
    pub alternatives: Vec<String>,
    /// Minimum hop count `m` (`0` enables the reflexive identity diagonal).
    pub min: usize,
    /// Maximum hop count `k` (PUBLIC, finite, `>= min`).
    pub max: usize,
}

impl HiddenBoundedPath {
    /// `(p){min,max}` over a single predicate.
    pub fn range(predicate: impl Into<String>, min: usize, max: usize) -> Self {
        HiddenBoundedPath {
            alternatives: vec![predicate.into()],
            min,
            max,
        }
    }

    /// `(p){k}` — exactly-`k` over a single predicate (mirrors
    /// [`eval_exact_k_chain_hidden`] but on this union machinery).
    pub fn exact(predicate: impl Into<String>, k: usize) -> Self {
        Self::range(predicate, k, k)
    }

    /// `(p?)` — the optional path, i.e. `{0,1}`.
    pub fn optional(predicate: impl Into<String>) -> Self {
        Self::range(predicate, 0, 1)
    }

    /// `(p_1 | … | p_a){min,max}` over an alternation of predicates.
    pub fn alternation(
        alternatives: impl IntoIterator<Item = impl Into<String>>,
        min: usize,
        max: usize,
    ) -> Self {
        HiddenBoundedPath {
            alternatives: alternatives.into_iter().map(Into::into).collect(),
            min,
            max,
        }
    }
}

/// The chain bit of ONE edge-tuple against a FIXED predicate sequence `chain`: it is
/// the secret-shared AND of (a) every internal hop equality `[e_i.object == e_{i+1}.
/// subject]` ([`crate::compare::secure_equal_to_bit`], never opened — exactly
/// [`tuple_chain_bit`]) and (b) a PUBLIC predicate-match gate per hop
/// (`e_i.predicate == chain[i]`). The predicate gate is a fully public decision: a
/// tuple whose predicates do not match the alternation branch contributes a
/// constant shared `0` (no secure multiplication at all) and is OR-folded away.
/// The intermediate node KEYS are fed to the secure equality but never
/// reconstructed by the arithmetic.
fn predicated_tuple_chain_bit(
    backend: &ShamirBackend,
    source: &mut dyn DistributedRandomness,
    tuple: &[&PredicatedEdge],
    chain: &[String],
) -> Result<Vec<Share>, MpcError> {
    debug_assert_eq!(tuple.len(), chain.len());
    let n = backend.parties();
    // Public predicate gate first: if any hop's edge predicate does not match the
    // required alternation branch this tuple cannot serve this chain — the chain bit
    // is the constant shared 0 (no crypto needed, fully public decision).
    for (edge, pred) in tuple.iter().zip(chain.iter()) {
        if &edge.predicate != pred {
            return Ok(const_sharing(n, Fp::zero()));
        }
    }
    // Internal-hop equalities, secret-shared and never opened — identical to the
    // exactly-k chain's `tuple_chain_bit`. Length-1 has no internal hop → shared 1.
    let mut bit = const_sharing(n, Fp::one());
    for w in tuple.windows(2) {
        let hop = secure_equal_to_bit_through(source, w[0].object.key, w[1].subject.key)?;
        bit = secret_and(source, &bit, &hop)?;
    }
    Ok(bit)
}

/// Fail-closed parameter checks for the bounded hidden path. `min <= max`, non-empty
/// alternation, `n >= 2t+1` (the AND/OR chain needs degree reduction), and the total
/// enumerated edge-tuples across the whole union — `Σ_ℓ (a^ℓ · |E|^ℓ)` — must be
/// within [`MAX_CHAIN_TUPLES`], all from PUBLIC parameters before any crypto runs
/// (the same public, statically-computable guard discipline as the exactly-k
/// operator).
fn check_bounded_params(
    backend: &ShamirBackend,
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
) -> Result<(), MpcError> {
    if form.alternatives.is_empty() {
        return Err(MpcError::Protocol(
            "hidden bounded path step has no predicates (empty alternation)".into(),
        ));
    }
    if form.min > form.max {
        return Err(MpcError::Protocol(format!(
            "hidden bounded path {{{},{}}} has min > max",
            form.min, form.max
        )));
    }
    // Hop counts are capped before evaluation — and before the Σ_ℓ accumulation
    // below, whose own cost is quadratic in `form.max`.
    if form.max > MAX_HOP_COUNT {
        return Err(MpcError::Protocol(format!(
            "hidden bounded path {{{},{}}} exceeds the MAX_HOP_COUNT cap of {} — refusing (the \
             union runs one fixed-length chain family per length up to the upper bound, so the \
             hop count is bounded before the enumeration is sized)",
            form.min, form.max, MAX_HOP_COUNT
        )));
    }
    let t = backend.threshold();
    if backend.parties() < 2 * t + 1 {
        return Err(MpcError::Protocol(format!(
            "hidden bounded path needs n >= 2t+1 for the AND/OR chain's degree reduction \
             (n={}, t={t})",
            backend.parties()
        )));
    }
    // Total tuple work = Σ_ℓ (#chains_ℓ · |E|^ℓ); #chains_ℓ = a^ℓ. Compute in closed
    // form (PUBLIC), reject on overflow or over-cap BEFORE any allocation/crypto.
    let m = edges.len() as u64;
    let a = form.alternatives.len() as u64;
    let mut total_tuples: u64 = 0;
    for length in form.min.max(1)..=form.max {
        let mut chains: u64 = 1;
        let mut per_chain: u64 = 1;
        let mut overflowed = false;
        for _ in 0..length {
            match (chains.checked_mul(a), per_chain.checked_mul(m)) {
                (Some(c), Some(p)) => {
                    chains = c;
                    per_chain = p;
                }
                _ => {
                    overflowed = true;
                    break;
                }
            }
        }
        let length_tuples = if overflowed {
            None
        } else {
            chains.checked_mul(per_chain)
        };
        match length_tuples.and_then(|v| total_tuples.checked_add(v)) {
            Some(v) => total_tuples = v,
            None => return oversized_bounded(edges, form),
        }
    }
    if total_tuples > MAX_CHAIN_TUPLES {
        return oversized_bounded(edges, form);
    }
    Ok(())
}

/// The controlled over-cap refusal shared by `check_bounded_params`'s overflow and
/// over-limit branches.
fn oversized_bounded<T>(edges: &PredicatedEdges, form: &HiddenBoundedPath) -> Result<T, MpcError> {
    Err(MpcError::Protocol(format!(
        "hidden bounded path {{{},{}}} over |E|={} edges and a {}-way alternation would \
         enumerate more than the MAX_CHAIN_TUPLES cap of {} edge-tuples — refusing \
         (CPU/round denial-of-service)",
        form.min,
        form.max,
        edges.len(),
        form.alternatives.len(),
        MAX_CHAIN_TUPLES,
    )))
}

/// Build one [`Candidate`] per DISTINCT disclosed endpoint pair, OR-folding the chain
/// bits of EVERY (length, alternation-branch, edge-tuple) that reaches that pair into
/// one secret-shared connected-bit. This is the design §2.2–§2.5 union with the
/// cross-length OR-fold dedup: a pair reached by a 2-hop AND a 4-hop chain (or by two
/// alternation branches) yields ONE candidate with connected-bit `1`, and **which
/// length connected it is not separately opened or encoded in the result** (§1.2
/// length-revealing prohibition).
///
/// When `form.min == 0` the reflexive length-0 diagonal `(x, x)` is added for every
/// node in the domain with the constant shared `1` chain bit (design §2.3 — every node
/// reaches itself by the empty path), OR-folded in exactly like a length-ℓ chain so a
/// node that is also reachable from itself by a longer chain still appears once.
fn build_bounded_candidates(
    backend: &ShamirBackend,
    source: &mut dyn DistributedRandomness,
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
) -> Result<Vec<Candidate>, MpcError> {
    let n = backend.parties();
    // disclosed endpoint key → (a term, b term, OR-folded connected bit).
    let mut by_pair: BTreeMap<String, (Term, Term, Vec<Share>)> = BTreeMap::new();

    // OR this chain bit into the connected-bit of endpoint pair (a, b).
    fn fold(
        source: &mut dyn DistributedRandomness,
        by_pair: &mut BTreeMap<String, (Term, Term, Vec<Share>)>,
        a: Term,
        b: Term,
        bit: Vec<Share>,
    ) -> Result<(), MpcError> {
        let key = endpoint_key(&a, &b);
        match by_pair.get_mut(&key) {
            Some((_, _, acc)) => {
                let folded = secret_or(source, acc, &bit)?;
                *acc = folded;
            }
            None => {
                by_pair.insert(key, (a, b, bit));
            }
        }
        Ok(())
    }

    // Length-0 reflexive diagonal (design §2.3) over the DECLARED public node
    // domain (SPARQL 1.1 §18.4), not the edge relation's own endpoints.
    if form.min == 0 {
        for node in edges.node_domain()? {
            fold(
                source,
                &mut by_pair,
                node.clone(),
                node.clone(),
                const_sharing(n, Fp::one()),
            )?;
        }
    }

    // Each length ℓ in max(min,1)..=max: union of the a^ℓ predicate chains, each a
    // base-|E| odometer over the ℓ-tuples of edges. All OR-folded per endpoint pair.
    let m = edges.len();
    if m > 0 {
        for length in form.min.max(1)..=form.max {
            // The `a^length` fixed predicate sequences, enumerated LAZILY: one
            // chain at a time from a single reusable buffer, so the peak memory
            // is one chain's worth of IRIs rather than every chain's
            // (`ChainOdometer`). The order is the materialized build's order, so
            // the per-chain draw order below is unchanged.
            let mut chains = ChainOdometer::new(&form.alternatives, length);
            while let Some(chain) = chains.next_chain() {
                let mut idx = vec![0usize; length];
                loop {
                    let tuple: Vec<&PredicatedEdge> =
                        idx.iter().map(|&i| &edges.edges[i]).collect();
                    let bit = predicated_tuple_chain_bit(backend, source, &tuple, chain)?;
                    let a_term = tuple[0].subject.term.clone();
                    let b_term = tuple[length - 1].object.term.clone();
                    fold(source, &mut by_pair, a_term, b_term, bit)?;

                    // Advance the base-|E| odometer (least-significant = idx[len-1]).
                    let mut pos = length;
                    let done = loop {
                        if pos == 0 {
                            break true;
                        }
                        pos -= 1;
                        idx[pos] += 1;
                        if idx[pos] < m {
                            break false;
                        }
                        idx[pos] = 0;
                    };
                    if done {
                        break;
                    }
                }
            }
        }
    }

    Ok(by_pair
        .into_values()
        .map(|(a, b, bit)| Candidate {
            payload: vec![Some(a), Some(b)],
            matched: MatchBit::SecretShared(bit),
        })
        .collect())
}

/// Evaluate a bounded HIDDEN-intermediate property path `?a (step){min,max} ?b` and
/// return the disclosed endpoint pairs as a [`PartialResult`] with
/// `vars == [?__pp_a, ?__pp_b]`. Covers `{1,k}`, `{0,k}` (reflexive), `p?` (`{0,1}`),
/// and per-hop alternation `(p_1 | … | p_a)`, built as a union of fixed-length
/// predicate chains over the SAME secret edge set, OR-folded to one connected-bit per
/// distinct endpoint pair (set semantics; the realized length is not separately
/// opened or encoded in the result, design
/// §1.2/§2.2–§2.5). Reuses the exactly-`k` chain's secret-shared equality + AND/OR
/// chaining and the landed oblivious output path verbatim.
///
/// `bound` is the public padded bound `B`; it must cover the distinct-endpoint-pair
/// candidate count (fail-closed BEFORE any draw — never truncates a match).
///
/// **How `B` is decided, and what the decision discloses.** `B` is first compared
/// against [`public_bounded_candidate_pair_bound`], an upper bound from the PUBLIC
/// input sizes. A `B` at or above it is accepted on those sizes alone: the distinct
/// endpoint-pair count `D` is not even computed, so the DECISION and its error text
/// rest on public sizes only. A `B` below it is accepted exactly when `D <= B`, so
/// success-or-refusal discloses that comparison; and because a refusal draws
/// nothing, repeated calls with different `B` recover `D` exactly by binary search.
/// The search only pins `D` up to
/// [`crate::oblivious_join::MAX_OUTPUT_BOUND`], since a larger `B` is refused by the
/// cap before either branch; past that the caller learns only that `D` is larger.
/// Either way the refusal message names only `B` and the public bound — and NEITHER
/// branch hides `D` from a successful run. The grouping keys every enumerated tuple
/// by its endpoint pair, read in the clear: in the model as built one process
/// simulates every party, so the grouping evaluator sees the ordered candidate pair
/// sequence. In a distributed implementation that supplies every evaluating party
/// with the ordered plaintext endpoint terms, or with this construction's
/// data-dependent accumulator wiring, every such party can determine the fold
/// positions. This crate implements a single-process simulation and does not
/// establish a per-party input or wiring-distribution boundary.
/// For a bounded form the tuple total is
/// `T = Σ_{ℓ=max(min,1)}^{max} a^ℓ·|E|^ℓ` (empty sum = 0), with `a` the alternation
/// width; length-zero entries are counted separately by `N₀`. The OR count is
/// `T + N₀ − D` when `min == 0`, the reflexive diagonal folding in one entry per
/// node of the `N₀`-node DECLARED public node domain, and
/// `T − D` otherwise. The output then pads from `D` to `B`, so the multiplication
/// count carries `D` regardless. Hiding `D` would need oblivious grouping with a
/// uniform schedule, which is not built; see the module docs for the transport note
/// and the limits of this account. A public-half refusal happens before any
/// candidate exists, and a tight-bound refusal runs the data-dependent projection
/// but no build — neither leaves a draw schedule.
///
/// **The public bound can be out of reach here.**
/// [`public_bounded_candidate_pair_bound`] sums the arms the form admits, and for
/// inputs `check_bounded_params` accepts that sum can exceed
/// [`crate::oblivious_join::MAX_OUTPUT_BOUND`] — a large enough `|E|` on the
/// `|E|` arm of `{0,1}`/`{1,1}`, or on the `|E|^2` arm of any form reaching
/// length 2, with the declared domain's size added on top when `min == 0`. No `B` is
/// then both at or above the public bound and within `MAX_OUTPUT_BOUND`, so such a
/// call can only run on the data-dependent branch, with the comparison that branch
/// discloses.
///
/// One fresh dealer covers the union stage AND the output
/// stage: this is `eval_bounded_path_hidden_slots_through` on that one dealer,
/// narrowed to the disclosed rows — a single code path, so nothing here can mint a
/// second dealer. Security tier is identical to [`eval_exact_k_chain_hidden`]:
/// honest-majority, semi-honest, no new assumption.
pub fn eval_bounded_path_hidden(
    backend: &ShamirBackend,
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
    bound: usize,
) -> Result<PartialResult, MpcError> {
    let (slots, _cost) =
        eval_bounded_path_hidden_slots_through(&mut backend.dealer(), backend, edges, form, bound)?;
    Ok(partial_result_from_slots(slots, chain_out_vars()))
}

/// Like [`eval_bounded_path_hidden`] but returns the raw oblivious output (the `B`
/// shuffled slots + modelled cost) — the transcript-level view that witnesses the
/// revealed slot count is exactly the public `B`.
pub fn eval_bounded_path_hidden_slots(
    backend: &ShamirBackend,
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    eval_bounded_path_hidden_slots_through(&mut backend.dealer(), backend, edges, form, bound)
}

/// [`eval_bounded_path_hidden_slots`] with the union stage's randomness drawn
/// through the [`DistributedRandomness`] seam (sq-yyro, issue #6117). Same routing,
/// source checks and fail-closed behaviour as
/// `eval_exact_k_chain_hidden_slots_through`, including the output stage drawn
/// from the SAME `source`; the single-dealer simulation's revealed multiset and
/// cost are unchanged.
pub(crate) fn eval_bounded_path_hidden_slots_through(
    source: &mut dyn DistributedRandomness,
    backend: &ShamirBackend,
    edges: &PredicatedEdges,
    form: &HiddenBoundedPath,
    bound: usize,
) -> Result<ObliviousOutput, MpcError> {
    require_source_matches(source, backend)?;
    check_bounded_params(backend, edges, form)?;
    // Both halves of the bound contract before any draw, public half first, exactly
    // as in the exactly-k entry: a `B` at or above `public_bounded_candidate_pair_bound`
    // is accepted on the edge count and the form's public min/max alone, and the
    // union projection below is never run; a tighter `B` is checked against the
    // exact union, which is the materialised key set rather than a sum of the arms,
    // so no `B` the data allows is refused. The stage re-checks after the build.
    // As above, the public-first order moves the count out of the decision, not out
    // of the schedule.
    crate::oblivious_join::check_bound_public_first(
        "hidden bounded path",
        bound,
        public_bounded_candidate_pair_bound(edges.len(), edges.declared_domain_len(), form)?,
        || projected_bounded_candidate_count(edges, form),
    )?;
    // The output stage's public-shape caps (declared widths, party count, aggregate
    // slot work) at THIS operator's entry, before a candidate is built or a share
    // drawn. Closed forms on public numbers, so the answer is the stage's own; the
    // stage re-checks.
    let widths = PayloadWidths::from(2);
    // The width pass over the edge endpoints AND the declared node domain — a
    // superset of the terms the candidates will carry, read before any draw.
    let resolved = widths.resolve(edges.payload_terms())?;
    crate::oblivious_join::check_output_public_shape(
        &resolved,
        bound,
        backend.parties(),
        backend.threshold(),
    )?;
    let candidates = build_bounded_candidates(backend, source, edges, form)?;
    oblivious_set_output_through(source, backend, &candidates, widths, bound)
}

/// Planner guard + cost-model wiring for the hidden bounded property-path operator
/// (`sq-py8h.5`): reject statically-large unrolls, refuse a hidden UNBOUNDED path
/// fail-closed via [`MpcError::NoBackendSatisfies`], and emit the operator's modelled
/// communication cost through [`crate::metrics::CommCounter`].
pub mod planner;

#[cfg(test)]
mod bounded_tests;
#[cfg(test)]
mod tests;
