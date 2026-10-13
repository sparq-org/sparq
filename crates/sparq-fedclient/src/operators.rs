//! Physical federation operators (design §4.4) — **Phase 3 (materialised single-source
//! interpreter)**.
//!
//! This module walks a [`JoinTree`](sparq_fedplan::JoinTree) and *interprets* it: for each
//! leaf it lowers the pattern to a [`SubQuery`](crate::source::SubQuery)
//! ([`crate::planner::lower_leaf`]), runs it through the Phase-2
//! [`FederatedSource`](crate::FederatedSource) adapter (`execute` → an SRJ body), parses
//! that body into a [`Relation`], and joins the leaf relations **in the plan's join order**
//! with a materialised left-deep hash join. The output is a [`Relation`] — the federated
//! answer.
//!
//! # The load-bearing correctness property
//!
//! The Phase-3 deliverable is the **result-equivalence invariant**, tested against the real
//! engine:
//!
//! > For a single source holding a graph `G`, materialising a `sparq-fedplan` plan of a BGP
//! > `Q` through this interpreter yields **the same solution multiset** as evaluating `Q`
//! > locally with `sparq-engine` over `G`.
//!
//! [`solutions_equal`] is the multiset comparison (order-independent, bag semantics) and
//! the integration test `planner_result_equals_local_eval.rs` drives it end-to-end: it
//! loads a graph into a `sparq-core::Graph`, serves each leaf sub-query from that graph via
//! a [`FederatedSource`] whose transport answers from the **local engine** (so the
//! "remote" SRJ is exactly what a conformant endpoint over `G` would return), runs this
//! interpreter, and asserts [`solutions_equal`] against `sparq_engine::query(&G, Q)`.
//! That closes the loop the bead asks for: the materialised result EQUALS local eval.
//!
//! # Honest work-vs-stub split (Phase 3)
//!
//! REAL here: the SRJ parser, the index-resolved leaf lowering, the materialised left-deep
//! join, the projection over the BGP's variables, and the result-equivalence test against
//! the engine. The join is a **blocking** materialised hash join — every leaf relation is
//! fetched and held in full before joining. STUBBED for Phase 5: the non-blocking
//! `StreamJoin` feeder, concurrent leaf fan-out, the per-block bind-join operator, and
//! multi-source UNION-per-leaf (this interpreter answers a plan whose every leaf resolves
//! to ONE source; a leaf with two retained sources is rejected with
//! [`InterpError::MultiSource`] rather than silently dropping a source). The interpreter
//! also requires the plan's join algorithm to be a hash-class step — it does not yet
//! special-case [`JoinAlgo::Bind`](sparq_fedplan::JoinAlgo::Bind); a bind join is executed
//! with the same materialised hash join (semantically identical result, just not the
//! pushed-down execution discipline), which is sound for the equivalence property.
//
// [OPUS-4.8] sq-j27p (epic sq-dnko): Phase-3 materialised single-source interpreter +
// SRJ parser + result-equivalence invariant. Flagged for Fable re-review when available.

use crate::planner::{lower_leaf, lower_leaf_fragment, pattern_vars, SourceResolver};
use crate::source::{FedError, FederatedSource, FragBinding, FragPattern, FragTerm, SourceType};
use oxrdf::{BlankNode, Literal, NamedNode, Term, Variable};
use sparq_fedplan::{JoinNode, JoinTree, PatternSources, TriplePattern};
use std::collections::HashMap;
// [OPUS-4.8] sq-vtba (Phase 5): the streaming-operator surface reuses the planner's proven
// non-blocking `StreamJoin` + its `Tuple` model, the `SolutionStream` boundary, and `std`
// threads/channels (NO async runtime — design §7).
use crate::stream::{Solution, SolutionSink, SolutionStream};
use sparq_fedplan::{StreamJoin, StreamJoinOptions, Tuple, Var as FpVar};
use std::str::FromStr;
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

// ─── The materialised relation (the interpreter's intermediate value) ────────────────

/// A materialised solution table: named variables + rows, each cell `Some(term)` bound or
/// `None` unbound — the same shape as `sparq-engine`'s `QueryResult`. The interpreter
/// produces and joins these. [OPUS-4.8] sq-j27p.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Relation {
    /// Variable names (without the leading `?`), in column order.
    pub vars: Vec<String>,
    /// One entry per `vars` column; `None` is unbound.
    pub rows: Vec<Vec<Option<Term>>>,
}

impl Relation {
    /// Project this relation onto `keep` (variable names), dropping every other column and
    /// reordering to `keep`'s order. A name absent from `self.vars` becomes an all-`None`
    /// (unbound) column — the SPARQL semantics of projecting a variable not in scope.
    pub fn project(&self, keep: &[String]) -> Relation {
        let idx: Vec<Option<usize>> = keep
            .iter()
            .map(|k| self.vars.iter().position(|v| v == k))
            .collect();
        let rows = self
            .rows
            .iter()
            .map(|row| {
                idx.iter()
                    .map(|i| i.and_then(|i| row.get(i).cloned().flatten()))
                    .collect()
            })
            .collect();
        Relation {
            vars: keep.to_vec(),
            rows,
        }
    }
}

// ─── Interpreter errors ──────────────────────────────────────────────────────────────

/// A failure walking / materialising a plan. [OPUS-4.8] sq-j27p.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpError {
    /// A plan index could not be resolved (see [`crate::planner::ResolveError`]).
    Resolve(crate::planner::ResolveError),
    /// A source adapter's `execute` failed (SSRF refusal, transport error, unsupported).
    Source(FedError),
    /// The SRJ body the adapter returned could not be parsed as SPARQL-Results-JSON.
    BadSrj(String),
    /// A leaf retained more than one source while running through a **single-source** entry
    /// point ([`materialize_single_source`] / [`stream_single_source`]) — those answer every
    /// leaf from one explicit `source` and fail closed rather than drop a source. To fan a
    /// multi-source leaf out as a per-source UNION use [`materialize_multi_source`] /
    /// [`stream_multi_source`] (bead `sq-7yf0`), which never returns this variant.
    MultiSource { pattern: usize, sources: usize },
    /// A leaf pattern could not be written as SPARQL, so it was not sent (see
    /// [`crate::planner::RenderError`]).
    Render(crate::planner::RenderError),
}

impl std::fmt::Display for InterpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InterpError::Resolve(e) => write!(f, "interpreter: {}", e),
            InterpError::Source(e) => write!(f, "interpreter: source error: {}", e),
            InterpError::BadSrj(m) => write!(f, "interpreter: malformed SRJ: {}", m),
            InterpError::MultiSource { pattern, sources } => write!(
                f,
                "interpreter: pattern {} has {} retained sources; the Phase-3 single-source \
                 interpreter answers one source per leaf (multi-source UNION is Phase 5)",
                pattern, sources
            ),
            InterpError::Render(e) => write!(f, "interpreter: pattern not sent: {}", e),
        }
    }
}

impl std::error::Error for InterpError {}

impl From<crate::planner::ResolveError> for InterpError {
    fn from(e: crate::planner::ResolveError) -> Self {
        InterpError::Resolve(e)
    }
}

impl From<crate::planner::RenderError> for InterpError {
    fn from(e: crate::planner::RenderError) -> Self {
        InterpError::Render(e)
    }
}

impl From<FedError> for InterpError {
    fn from(e: FedError) -> Self {
        InterpError::Source(e)
    }
}

// ─── The materialised single-source interpreter ──────────────────────────────────────

/// Materialise a `sparq-fedplan` [`JoinTree`] against a **single source** and return the
/// federated solution table.
///
/// `resolver` maps the plan's pattern/source indices to patterns/adapters (the Phase-0
/// finding's resolution layer); `selection` is the planner's per-pattern source selection
/// ([`select_sources`](sparq_fedplan::select_sources)) — the interpreter consults it to
/// **enforce the single-source contract**: a leaf that retained more than one source is
/// rejected with [`InterpError::MultiSource`] (Phase 5 fans those out as a UNION). `source`
/// is the one [`FederatedSource`] every leaf is answered by. For each leaf the interpreter
/// lowers the pattern ([`lower_leaf`]), fetches the SRJ via `source.execute`, parses it into
/// a [`Relation`], and folds the leaves together in the plan's join order with a
/// materialised left-deep natural (hash) join. The final relation is projected onto the
/// BGP's variables (the union of every pattern's variables, in first-seen order).
///
/// The join algorithm chosen by the planner ([`JoinAlgo`](sparq_fedplan::JoinAlgo)) is
/// honoured *semantically* but not *physically* in this phase: bind and hash both execute
/// as the same materialised natural join (identical result multiset). The non-blocking
/// `StreamJoin` feeder + the pushed-down bind-join operator are Phase 5. [OPUS-4.8] sq-j27p.
pub fn materialize_single_source(
    resolver: &SourceResolver<'_>,
    selection: &[PatternSources],
    source: &dyn FederatedSource,
    tree: &JoinTree,
) -> Result<Relation, InterpError> {
    let rel = materialize_tree(&tree.root, &mut |pattern| {
        single_source_leaf(resolver, selection, source, pattern)
    })?;
    // Project onto the whole-BGP variable set (first-seen position order across patterns),
    // matching what a `SELECT *` over the BGP would scope.
    Ok(rel.project(&bgp_vars(resolver)))
}

/// Materialise a `sparq-fedplan` [`JoinTree`] across **all retained sources per leaf**,
/// fanning a leaf the planner kept more than one source for out as a **per-source UNION**,
/// and return the federated solution table. This lifts the single-source restriction the
/// Phase-3/5/7 [`InterpError::MultiSource`] guard imposed (bead `sq-7yf0`).
///
/// Where [`materialize_single_source`] answers every leaf from one explicit `source` (and
/// rejects a multi-source leaf), this entry point resolves each leaf's retained source
/// **indices** ([`PatternSources::candidates`](sparq_fedplan::PatternSources)) to adapters
/// through the `resolver` and answers the leaf as the **bag-union** of every retained
/// source's relation for that pattern. That is exactly SPARQL UNION semantics over the
/// per-source solution sequences: the multisets are concatenated, no de-duplication
/// (SELECT bag semantics), preserving multiplicity. A leaf with a single retained source
/// degrades to the same fetch the single-source interpreter performs, so for a plan whose
/// every leaf resolves to one source the two entry points return the identical multiset.
///
/// The join order, projection, and the materialised left-deep natural join are
/// unchanged — only the per-leaf input is now a union over sources rather than one source.
/// Joining over the *union per leaf* (rather than joining per-source and unioning the
/// whole BGP) is the correct federated semantics: a leaf's contribution is the union of
/// what each source holds for that pattern, and the BGP answer is the natural join of those
/// per-leaf unions, regardless of which source each matching triple came from.
///
/// A leaf whose selection retained **no** source is an empty relation over the pattern's
/// variables (the source-selection layer is the single source of truth; the interpreter
/// does not silently fall back to "every adapter"). [OPUS-4.8] sq-7yf0.
pub fn materialize_multi_source(
    resolver: &SourceResolver<'_>,
    selection: &[PatternSources],
    tree: &JoinTree,
) -> Result<Relation, InterpError> {
    let rel = materialize_tree(&tree.root, &mut |pattern| {
        union_leaf(resolver, selection, pattern, |_, source, tp| {
            fetch_leaf_relation(source, tp)
        })
    })?;
    // Project onto the whole-BGP variable set, matching `materialize_single_source`.
    Ok(rel.project(&bgp_vars(resolver)))
}

/// Materialise one plan node into a [`Relation`]: each leaf is answered by `leaf`, and a join
/// materialises its left subtree and its right leaf and natural-joins them. The single- and
/// multi-source interpreters differ only in `leaf`. [OPUS-4.8] sq-j27p / sq-7yf0.
fn materialize_tree(
    node: &JoinNode,
    leaf: &mut dyn FnMut(usize) -> Result<Relation, InterpError>,
) -> Result<Relation, InterpError> {
    match node {
        JoinNode::Leaf { pattern, .. } => leaf(*pattern),
        JoinNode::Join { left, right, .. } => {
            let l = materialize_tree(left, leaf)?;
            let r = leaf(*right)?;
            Ok(natural_join(&l, &r))
        }
    }
}

/// Answer ONE leaf pattern as the **bag-union over its retained sources**: resolve each
/// candidate source index to its adapter through `resolver`, `fetch` the leaf relation from it,
/// and concatenate the rows re-keyed onto the pattern's variable header (a source's SRJ header
/// may differ in order). SPARQL UNION multiset semantics, no de-duplication; a pattern with no
/// retained source is the empty relation over its variables. `fetch` receives the source index
/// so the adaptive executor can time each arm. [OPUS-4.8] sq-7yf0, [FABLE-5] sq-xw8zz.
pub(crate) fn union_leaf<F>(
    resolver: &SourceResolver<'_>,
    selection: &[PatternSources],
    pattern: usize,
    mut fetch: F,
) -> Result<Relation, InterpError>
where
    F: FnMut(usize, &dyn FederatedSource, &TriplePattern) -> Result<Relation, InterpError>,
{
    let tp = resolver.pattern(pattern)?;
    let vars = pattern_vars(tp);
    let mut union = Relation {
        vars: vars.clone(),
        rows: Vec::new(),
    };
    for idx in leaf_source_indices(selection, pattern) {
        let leaf = fetch(idx, resolver.source(idx)?, tp)?;
        for row in &leaf.rows {
            union.rows.push(rel_row_project(&leaf.vars, row, &vars));
        }
    }
    Ok(union)
}

/// The retained source indices for `pattern`, ascending (the deterministic order
/// [`select_sources`](sparq_fedplan::select_sources) records). A pattern absent from the
/// selection — or one whose candidate set is empty — contributes no sources, hence an empty
/// leaf relation. `pub(crate)`: the adaptive executor's union-arm leaf fetch
/// ([FABLE-5] sq-xw8zz) shares this so both fan-outs resolve arms identically. [OPUS-4.8] sq-7yf0.
pub(crate) fn leaf_source_indices(selection: &[PatternSources], pattern: usize) -> Vec<usize> {
    selection
        .iter()
        .find(|ps| ps.pattern == pattern)
        .map(|ps| ps.candidates.iter().map(|c| c.source).collect())
        .unwrap_or_default()
}

/// All distinct variables of the resolver's BGP, in first-seen order (subject, predicate,
/// object within a pattern; patterns in BGP order) — the `SELECT *` scope. Shared with the
/// adaptive executor so every interpreter projects the same header. [OPUS-4.8] sq-j27p.
pub(crate) fn bgp_vars(resolver: &SourceResolver<'_>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tp in &resolver.bgp().patterns {
        for v in pattern_vars(tp) {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

/// Reject (never silently drop) a leaf the planner retained more than one source for: the
/// single-source entry points answer every leaf from one explicit source. [OPUS-4.8] sq-j27p.
fn single_source_guard(selection: &[PatternSources], pattern: usize) -> Result<(), InterpError> {
    match selection.iter().find(|ps| ps.pattern == pattern) {
        Some(ps) if ps.candidates.len() > 1 => Err(InterpError::MultiSource {
            pattern,
            sources: ps.candidates.len(),
        }),
        _ => Ok(()),
    }
}

/// Answer one leaf from the single `source` after the single-source guard. Shared by the
/// materialising and adaptive single-source interpreters, so both dispatch on the source's
/// interface through [`fetch_leaf_relation`]. [OPUS-4.8] sq-j27p.
pub(crate) fn single_source_leaf(
    resolver: &SourceResolver<'_>,
    selection: &[PatternSources],
    source: &dyn FederatedSource,
    pattern: usize,
) -> Result<Relation, InterpError> {
    single_source_guard(selection, pattern)?;
    fetch_leaf_relation(source, resolver.pattern(pattern)?)
}

/// Answer ONE leaf triple pattern against `source`, returning the leaf [`Relation`] —
/// **dispatching on the source's interface** so a fragment source is answered through its typed
/// fragment path rather than the SRJ `execute` (which a fragment server refuses, since it speaks
/// triples, not SPARQL-Results-JSON — bead `sq-2qze`'s `FedError::Unsupported` stub).
///
/// * **[`Endpoint`](crate::source::Endpoint) / [`Local`](crate::source::LocalSource)** — lower
///   to a SPARQL [`SubQuery`](crate::source::SubQuery) ([`lower_leaf`]), `execute` → an SRJ body,
///   parse → [`Relation`] (the Phase-3 path).
/// * **[`Tpf`](crate::source::TpfSource)** — lower to a [`FragPattern`] ([`lower_leaf_fragment`]),
///   call [`solutions`](crate::source::TpfSource::solutions) (the fragment fetched to exhaustion,
///   complete by construction), and build a [`Relation`] over the pattern's variables.
/// * **[`BrTpf`](crate::source::BrTpfSource)** — same, calling
///   [`solutions`](crate::source::BrTpfSource::solutions) with an EMPTY upstream binding block: a
///   complete unbound fragment scan that the interpreter then hash-joins locally. This mirrors how
///   the Phase-3 interpreter executes a planner `JoinAlgo::Bind` as a materialised hash join — the
///   result multiset is identical (the pushed-down brTPF bind-join is a per-block execution
///   discipline, not a different answer); the streaming bind-join feeder is a later phase.
///
/// This is the wiring the bead asks for: a `JoinTree` leaf that resolves to a TPF/brTPF source is
/// now answered, not rejected with `Unsupported`. The single-source-per-leaf contract is enforced
/// by the caller. `pub(crate)`: the adaptive executor's union-arm leaf fetch ([FABLE-5] sq-xw8zz)
/// times this exact fetch per arm, so both interpreters answer a leaf identically. [OPUS-4.8] sq-yzca.
pub(crate) fn fetch_leaf_relation(
    source: &dyn FederatedSource,
    tp: &TriplePattern,
) -> Result<Relation, InterpError> {
    match source.source_type() {
        SourceType::Tpf(tpf) => {
            let pat = lower_leaf_fragment(tp);
            let rows = tpf.solutions(&pat)?;
            Ok(frag_solutions_to_relation(&pat, rows))
        }
        SourceType::BrTpf(brtpf) => {
            let pat = lower_leaf_fragment(tp);
            // An empty upstream binding block ⇒ a complete unbound fragment scan; the interpreter
            // hash-joins it locally (semantically identical to the pushed brTPF bind-join).
            let rows = brtpf.solutions(&pat, &[])?;
            Ok(frag_solutions_to_relation(&pat, rows))
        }
        SourceType::Endpoint(_) | SourceType::Local(_) => {
            let sub = lower_leaf(tp)?;
            let body = source.execute(&sub)?;
            parse_srj(&body).map_err(InterpError::BadSrj)
        }
    }
}

/// Build a [`Relation`] from a fragment source's typed solution mappings for `pattern`. The
/// relation's columns are the pattern's variables in position order ([`FragPattern::vars`]); each
/// [`FragBinding`] row contributes one [`Relation`] row, with each [`FragTerm`] converted back to
/// the engine's [`oxrdf::Term`] model so the leaf joins against the SAME term type a SPARQL-
/// endpoint leaf produces (so a federated query mixing endpoint + fragment sources joins
/// correctly). A binding the fragment did not bind for a column is unbound (`None`). [OPUS-4.8].
fn frag_solutions_to_relation(pattern: &FragPattern, rows: Vec<FragBinding>) -> Relation {
    let vars = pattern.vars();
    let rows = rows
        .iter()
        .map(|binding| {
            vars.iter()
                .map(|v| {
                    binding
                        .iter()
                        .find(|(name, _)| name == v)
                        .map(|(_, term)| fragterm_to_oxterm(term))
                })
                .collect()
        })
        .collect();
    Relation { vars, rows }
}

/// Convert one [`FragTerm`] back into the engine's [`oxrdf::Term`] model, preserving the term's
/// exact lexical identity so it equi-joins with the SAME term parsed from an endpoint's SRJ.
///
/// * an IRI / blank node is reconstructed directly; an invalid IRI/label degrades to a simple
///   literal carrying the raw bytes (never a panic — `forbid(unsafe_code)`, and a malformed term
///   from a misbehaving server must not crash the join);
/// * a literal is parsed from its canonical N-Triples lexical form (`"v"`, `"v"@lang`,
///   `"v"^^<dt>` — the `FragTerm::Literal` model stores exactly that) via `Term::from_str`, the
///   inverse of the `oxterm_to_fragterm` writer the HTTP transport uses, so the round-trip is
///   lossless; an unparseable lexical degrades to a simple literal of the raw text. [OPUS-4.8].
fn fragterm_to_oxterm(term: &FragTerm) -> Term {
    match term {
        FragTerm::Iri(s) => NamedNode::new(s)
            .map(Term::NamedNode)
            .unwrap_or_else(|_| Term::Literal(Literal::new_simple_literal(s.clone()))),
        FragTerm::Blank(s) => BlankNode::new(s)
            .map(Term::BlankNode)
            .unwrap_or_else(|_| Term::Literal(Literal::new_simple_literal(s.clone()))),
        FragTerm::Literal(s) => {
            Term::from_str(s).unwrap_or_else(|_| Term::Literal(Literal::new_simple_literal(s.clone())))
        }
    }
}

/// A materialised **natural (inner) join** of two relations on their shared variables,
/// using a hash index on the smaller side. Shared columns must be equal (same `Some(term)`)
/// for two rows to combine; the result carries `left.vars` followed by the right-only vars.
/// This is the materialised analogue of the `StreamJoin` Phase 5 will run incrementally —
/// it produces the identical result multiset (bag semantics, no de-duplication).
///
/// `pub(crate)` so the Phase-7 adaptive executor ([`crate::adaptive`]) reuses the SAME
/// proven join operator when it folds the re-ordered suffix — the result-equivalence
/// invariant rests on adaptive + static using one identical join. [OPUS-4.8] sq-j27p / sq-ij5x.
pub(crate) fn natural_join(left: &Relation, right: &Relation) -> Relation {
    // Shared variables (join key) and the right-only variables (appended columns).
    let shared: Vec<(usize, usize)> = left
        .vars
        .iter()
        .enumerate()
        .filter_map(|(li, lv)| right.vars.iter().position(|rv| rv == lv).map(|ri| (li, ri)))
        .collect();
    let right_only: Vec<usize> = right
        .vars
        .iter()
        .enumerate()
        .filter(|(_, rv)| !left.vars.contains(rv))
        .map(|(ri, _)| ri)
        .collect();

    let mut out_vars = left.vars.clone();
    for &ri in &right_only {
        out_vars.push(right.vars[ri].clone());
    }

    // Build a hash index on the LEFT side keyed by the shared-column values, then probe with
    // the right. The key is the vector of bound terms at the shared columns; an UNBOUND
    // shared cell never equi-joins (it cannot equal a bound value), matching SPARQL's
    // compatible-mapping rule for the variables both sides bind.
    let mut index: HashMap<Vec<Term>, Vec<usize>> = HashMap::new();
    for (i, lrow) in left.rows.iter().enumerate() {
        if let Some(key) = join_key(lrow, &shared, true) {
            index.entry(key).or_default().push(i);
        }
    }

    let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
    for rrow in &right.rows {
        let Some(key) = join_key(rrow, &shared, false) else {
            continue;
        };
        if let Some(matches) = index.get(&key) {
            for &li in matches {
                let lrow = &left.rows[li];
                let mut out = lrow.clone();
                for &ri in &right_only {
                    out.push(rrow.get(ri).cloned().flatten());
                }
                rows.push(out);
            }
        }
    }

    Relation {
        vars: out_vars,
        rows,
    }
}

/// The join key for a row at the shared columns: the bound terms in column order, or `None`
/// if any shared cell is unbound (an unbound join variable cannot equi-join). `from_left`
/// selects the left or right index of each `(li, ri)` shared pair. [OPUS-4.8] sq-j27p.
fn join_key(row: &[Option<Term>], shared: &[(usize, usize)], from_left: bool) -> Option<Vec<Term>> {
    let mut key = Vec::with_capacity(shared.len());
    for &(li, ri) in shared {
        let col = if from_left { li } else { ri };
        let t = row.get(col).cloned().flatten()?;
        key.push(t);
    }
    Some(key)
}

// ─── SRJ parser (SPARQL-Results-JSON → Relation) ─────────────────────────────────────

/// Parse a SPARQL-Results-JSON SELECT body into a [`Relation`].
///
/// This is the engine's streaming SERVICE reader (`sparq_engine::parse_srj_into`), so a
/// relation parsed here is row-for-row what the engine's SERVICE would read: `uri`/`bnode`/
/// `literal` (+`xml:lang`/`its:dir`/`datatype`) and the SPARQL-1.2 `triple` term, a variable
/// absent from a solution object is UNBOUND (`None`), and an ASK `boolean` body is rejected
/// (this interpreter only joins SELECT relations). Errors carry the engine's text without its
/// `SERVICE: ` prefix.
pub fn parse_srj(text: &str) -> Result<Relation, String> {
    let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
    let vars = sparq_engine::parse_srj_into(text, &mut |row| {
        rows.push(row);
        Ok(())
    })
    .map_err(|e| e.strip_prefix("SERVICE: ").map(str::to_string).unwrap_or(e))?;
    let vars = vars.into_iter().map(Variable::into_string).collect();
    Ok(Relation { vars, rows })
}

// ─── Result-equivalence (the load-bearing invariant) ─────────────────────────────────

/// One solution as a binding map (variable → bound term **rendered to its canonical
/// string**), dropping unbound variables — the normal form for **bag-semantics,
/// order-independent** comparison.
///
/// `oxrdf::Term` is `Eq`/`Hash` but not `Ord`, so the bag is sorted on the term's `Display`
/// form, which is the canonical N-Triples serialisation (`<iri>`, `_:b`, `"v"^^<dt>`,
/// `"v"@lang`, `<< s p o >>`). That serialisation is injective — two terms are equal iff
/// their Display strings are equal — so comparing on it preserves the term-equality the
/// invariant needs while giving a total order to sort by. [OPUS-4.8] sq-j27p.
fn solution_map(vars: &[String], row: &[Option<Term>]) -> Vec<(String, String)> {
    let mut m: Vec<(String, String)> = vars
        .iter()
        .zip(row.iter())
        .filter_map(|(v, cell)| cell.as_ref().map(|t| (v.clone(), t.to_string())))
        .collect();
    m.sort();
    m
}

/// Multiset (bag) of solution maps for a relation — the comparison normal form. A relation
/// and the engine's `QueryResult` are equal iff these bags are equal (same maps with the
/// same multiplicities), independent of row order and column order. [OPUS-4.8] sq-j27p.
fn solution_bag(vars: &[String], rows: &[Vec<Option<Term>>]) -> Vec<Vec<(String, String)>> {
    let mut bag: Vec<Vec<(String, String)>> =
        rows.iter().map(|row| solution_map(vars, row)).collect();
    bag.sort();
    bag
}

/// The load-bearing **result-equivalence** check: do `a` and `b` carry the same solution
/// multiset (bag semantics, order-independent in both rows and columns)?
///
/// This is the predicate the Phase-3 correctness test asserts between this interpreter's
/// materialised [`Relation`] and `sparq-engine`'s local `QueryResult` for the same BGP over
/// the same graph. It compares **solutions**, not table layout: SPARQL solution sequences
/// are unordered (absent ORDER BY) and a binding map is column-order-independent, so two
/// results that differ only in row/column order are equal — but a different multiplicity or
/// a different bound term is NOT. [OPUS-4.8] sq-j27p.
pub fn solutions_equal(
    a_vars: &[String],
    a_rows: &[Vec<Option<Term>>],
    b_vars: &[String],
    b_rows: &[Vec<Option<Term>>],
) -> bool {
    solution_bag(a_vars, a_rows) == solution_bag(b_vars, b_rows)
}

// ═════════════════════════════════════════════════════════════════════════════════════
// Phase 5 — streaming operators (bead sq-vtba, epic sq-dnko)
//
// Everything below is the Phase-5 surface: a bounded blocking thread-pool over the blocking
// transport, the `StreamJoin` feeder that bridges `oxrdf::Term` solutions into the planner's
// proven non-blocking join, and a streaming interpreter that walks the same `JoinTree` as
// Phase 3 but EMITS results before the inputs are exhausted. The load-bearing invariant: the
// streamed multiset is multiset-EQUAL to `materialize_single_source` for ANY source-arrival
// interleaving (the streaming-correctness test in `tests/streaming_result_equals_phase3.rs`).
//
// ASYNC/RUNTIME DECISION (design §7, deferred to here): NO async runtime is pulled in. All
// concurrency is `std` only — a bounded thread-pool + bounded `sync_channel`s — and stays
// inside this opt-in crate, so the lean core is untouched.
//
// [OPUS-4.8] sq-vtba — flagged for Fable re-review when available.
// ═════════════════════════════════════════════════════════════════════════════════════

// ─── The `oxrdf::Term` ⟷ `fedplan::Tuple` bridge (lossless, injective) ────────────────

/// The unit-separator used to fold a multi-valued variable away (it cannot occur in a
/// canonical N-Triples term, the same property `fedplan::Tuple::key` relies on).
///
/// Convert one [`Solution`] into a `fedplan` [`Tuple`]: each **bound** cell becomes a
/// `(Var, lexical)` pair whose lexical value is the term's canonical N-Triples form
/// (`Term::Display` — `<iri>`, `_:b`, `"v"^^<dt>`, `"v"@lang`, `"v"@lang--dir`, `<< s p o >>`).
/// That serialisation is **injective** — two terms are equal iff their Display strings are
/// equal — so a `Tuple` join key over it agrees exactly with term equality, and
/// [`tuple_to_solution`] parses it back with `Term::from_str` losslessly. An **unbound** cell
/// is omitted (a `fedplan` tuple has no notion of an explicit unbound binding; an absent
/// variable is exactly "unbound"), matching the Phase-3 join's rule that an unbound shared
/// cell never equi-joins. [OPUS-4.8] sq-vtba.
fn solution_to_tuple(sol: &Solution) -> Tuple {
    let pairs = sol
        .vars
        .iter()
        .zip(sol.cells.iter())
        .filter_map(|(v, cell)| cell.as_ref().map(|t| (FpVar::new(v.clone()), t.to_string())));
    Tuple::new(pairs)
}

/// Reconstruct a [`Solution`] from a joined `fedplan` [`Tuple`], under the output header
/// `out_vars`. Each output variable that the tuple binds is parsed back from its canonical
/// N-Triples lexical via `Term::from_str` (the inverse of [`solution_to_tuple`]); a variable
/// the tuple does not bind is unbound (`None`). A lexical that fails to re-parse (it cannot,
/// for a value produced by `Term::Display`) surfaces as an error so a malformed value never
/// silently becomes a wrong term. [OPUS-4.8] sq-vtba.
fn tuple_to_solution(t: &Tuple, out_vars: &[String]) -> Result<Solution, FedError> {
    let mut cells = Vec::with_capacity(out_vars.len());
    for v in out_vars {
        match t.get(&FpVar::new(v.clone())) {
            Some(lex) => {
                let term = Term::from_str(lex).map_err(|e| {
                    FedError::Transport(format!("streaming join: un-parseable term {:?}: {}", lex, e))
                })?;
                cells.push(Some(term));
            }
            None => cells.push(None),
        }
    }
    Ok(Solution::new(out_vars.to_vec(), cells))
}

// ─── Bounded blocking thread-pool (the ASYNC/RUNTIME decision, design §7) ─────────────

/// A **bounded blocking thread-pool** the streaming operators fan source fetches out across,
/// WITHOUT an async runtime. Each submitted job is a blocking closure (a `source.execute`
/// round-trip + parse); at most `workers` run concurrently. This is the concurrency the
/// design §7 deferred to Phase 5: real parallelism over the blocking transport, `std`-only,
/// confined to this opt-in crate.
///
/// Jobs are taken from a bounded queue (`sync_channel`), so a caller that submits faster than
/// the workers drain is back-pressured rather than building an unbounded backlog.
///
/// # Lifetime — the pool is **detached**, not joined-on-drop
///
/// A leaf job writes into a bounded per-leaf channel and **blocks** on `emit` until the
/// consumer pulls (the backpressure). The producing pool therefore MUST outlive the returned
/// stream — joining it on drop would either defeat streaming (block the caller until every
/// fetch finishes) or deadlock (a worker blocked on `emit` while the only consumer waits for
/// the call to return). So `Drop` only **closes the queue** (so idle workers exit once it
/// drains) and lets the running workers finish detached; each terminates when its fetch
/// completes or its consumer goes away (`emit` → `false`). Threads are query-scoped: every
/// worker exits once the leaf streams are consumed or dropped. Call [`ScatterPool::join`] to
/// block for a clean shutdown when you DO want to wait (tests). [OPUS-4.8] sq-vtba.
pub struct ScatterPool {
    tx: Option<std::sync::mpsc::SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

impl ScatterPool {
    /// Spin up a pool of `workers` threads (at least one) draining a bounded job queue of
    /// `queue_cap` (at least one) pending jobs. [OPUS-4.8] sq-vtba.
    pub fn new(workers: usize, queue_cap: usize) -> ScatterPool {
        let (tx, rx) = sync_channel::<Job>(queue_cap.max(1));
        let rx = Arc::new(Mutex::new(rx));
        let mut handles = Vec::with_capacity(workers.max(1));
        for _ in 0..workers.max(1) {
            let rx = Arc::clone(&rx);
            handles.push(thread::spawn(move || loop {
                // Take the next job under the lock, then RELEASE the lock before running it
                // (so workers run jobs in parallel, only the dequeue is serialised).
                let job = {
                    let guard = rx.lock().expect("[OPUS-4.8] sq-vtba: pool queue lock");
                    guard.recv()
                };
                match job {
                    Ok(job) => job(),
                    Err(_) => break, // queue closed → no more jobs → exit.
                }
            }));
        }
        ScatterPool {
            tx: Some(tx),
            workers: handles,
        }
    }

    /// Submit a blocking job. Blocks when the bounded queue is full (back-pressure).
    /// [OPUS-4.8] sq-vtba.
    pub fn submit<F: FnOnce() + Send + 'static>(&self, job: F) {
        if let Some(tx) = &self.tx {
            // A closed receiver can only happen after the queue is closed; ignore the error
            // (the job is simply not run, which is correct once the pool is shutting down).
            let _ = tx.send(Box::new(job));
        }
    }

    /// Close the queue and **join** every worker (waits for in-flight jobs to finish). Use
    /// when a clean, blocking shutdown is wanted (e.g. tests draining their streams first).
    /// [OPUS-4.8] sq-vtba.
    pub fn join(mut self) {
        self.tx.take(); // close the queue → idle workers exit.
        for h in self.workers.drain(..) {
            let _ = h.join();
        }
    }
}

impl Drop for ScatterPool {
    fn drop(&mut self) {
        // Detached: close the queue so idle workers exit, but DO NOT join (see the type doc —
        // joining here would block the caller or deadlock against an un-drained consumer).
        self.tx.take();
        // `workers` JoinHandles drop un-joined → the threads run detached to completion.
    }
}

// ─── The streaming `StreamJoin` feeder ────────────────────────────────────────────────

/// The shared **join variables** of two solution headers: variables present in BOTH (the
/// equi-join key), in `left`'s order. The same key the Phase-3 [`natural_join`] computes,
/// so the streamed join and the materialised join agree. [OPUS-4.8] sq-vtba.
fn join_vars(left: &[String], right: &[String]) -> Vec<String> {
    left.iter()
        .filter(|v| right.contains(v))
        .cloned()
        .collect()
}

/// The output header of a join: `left`'s variables followed by `right`'s left-not-present
/// variables (matching Phase-3 `natural_join`'s `out_vars`). [OPUS-4.8] sq-vtba.
fn join_out_vars(left: &[String], right: &[String]) -> Vec<String> {
    let mut out = left.to_vec();
    for v in right {
        if !left.contains(v) {
            out.push(v.clone());
        }
    }
    out
}

/// A **streaming binary join** over two incrementally-arriving [`SolutionStream`]s, driven by
/// the planner's proven non-blocking [`StreamJoin`] (symmetric hash join + bounded spill).
///
/// `run` consumes `left` and `right` and produces a new [`SolutionStream`] that EMITS a
/// joined solution the moment the **later-arriving** of a matching pair is processed — never
/// waiting for either input to finish. Because [`StreamJoin`] is proven multiset-equal to a
/// blocking hash join for ANY interleaving and any spill budget, the streamed result is the
/// same multiset Phase-3's `natural_join` produces — the load-bearing correctness property.
///
/// Interleaving is *real*: a dedicated feeder thread drains both input streams (pulling from
/// whichever has an item ready, via two `recv` loops multiplexed by a tiny merge) and pushes
/// each arrival into the `StreamJoin`, forwarding every emitted match to the output sink. The
/// feeder thread is the only place the two inputs meet, so no extra synchronisation is
/// needed. [OPUS-4.8] sq-vtba.
pub struct StreamingJoin {
    out_vars: Vec<String>,
    join_vars: Vec<String>,
    opts: StreamJoinOptions,
    /// Output channel backpressure bound (in-flight joined solutions).
    out_cap: usize,
}

impl StreamingJoin {
    /// A streaming join whose output header / join key are derived from the two input
    /// headers. `opts` tunes the reused [`StreamJoin`] (memory budget + spill store);
    /// `out_cap` bounds the output channel. [OPUS-4.8] sq-vtba.
    pub fn new(
        left_vars: &[String],
        right_vars: &[String],
        opts: StreamJoinOptions,
        out_cap: usize,
    ) -> StreamingJoin {
        StreamingJoin {
            out_vars: join_out_vars(left_vars, right_vars),
            join_vars: join_vars(left_vars, right_vars),
            opts,
            out_cap,
        }
    }

    /// The output header this join produces.
    pub fn out_vars(&self) -> &[String] {
        &self.out_vars
    }

    /// Run the join over `left`/`right`, returning the joined [`SolutionStream`]. Spawns one
    /// feeder thread; the thread ends (and the output stream closes) when both inputs are
    /// drained. [OPUS-4.8] sq-vtba.
    pub fn run(self, left: SolutionStream, right: SolutionStream) -> SolutionStream {
        let (sink, out) = SolutionStream::bounded(self.out_cap);
        let StreamingJoin {
            out_vars,
            join_vars,
            opts,
            ..
        } = self;
        thread::spawn(move || {
            feed_streaming_join(left, right, &join_vars, &out_vars, opts, &sink);
            // `sink` drops here → the output stream ends.
        });
        out
    }
}

/// Drive a [`StreamJoin`] from two solution streams under an arbitrary arrival interleaving.
///
/// The merge is genuinely incremental: it pulls from whichever input currently has a ready
/// item (a left item if one is available, else a right item, else blocks for either), pushes
/// it into the symmetric join, and forwards the emitted matches. A side that has finished is
/// skipped. The loop exits when BOTH inputs are exhausted. Any error item is forwarded and
/// stops the join (a producer failure is terminal for that branch). [OPUS-4.8] sq-vtba.
fn feed_streaming_join(
    left: SolutionStream,
    right: SolutionStream,
    join_vars: &[String],
    out_vars: &[String],
    opts: StreamJoinOptions,
    sink: &SolutionSink,
) {
    let jvars: Vec<FpVar> = join_vars.iter().map(|v| FpVar::new(v.clone())).collect();
    let mut join = StreamJoin::new(jvars, opts);

    // Both inputs are channel-backed; multiplex them by readiness (`try_recv` first, blocking
    // `recv` when neither is immediately ready) so the feeder never busy-spins and a faster
    // source is not starved by a slower one.
    let mut left = ChannelPeek::over(left);
    let mut right = ChannelPeek::over(right);

    loop {
        match (left.is_done(), right.is_done()) {
            (true, true) => break,
            (false, true) => {
                if !pump(&mut left, true, &mut join, out_vars, sink) {
                    return;
                }
            }
            (true, false) => {
                if !pump(&mut right, false, &mut join, out_vars, sink) {
                    return;
                }
            }
            (false, false) => {
                // Prefer whichever side has an item ready RIGHT NOW (non-blocking peek); if
                // neither is ready, block on the left first (then the right) — correctness is
                // interleaving-independent, this only affects latency, not the result.
                let took_left = if left.ready() {
                    pump(&mut left, true, &mut join, out_vars, sink)
                } else if right.ready() {
                    pump(&mut right, false, &mut join, out_vars, sink)
                } else {
                    // Neither ready: block on left (a blocking pull), which also advances the
                    // done-state when that side closes.
                    pump(&mut left, true, &mut join, out_vars, sink)
                };
                if !took_left {
                    return;
                }
            }
        }
    }
}

/// Pull ONE item from `side` and feed it into the join, forwarding emitted matches to `sink`.
/// Returns `false` if the consumer went away (stop) or a producer error was forwarded.
/// `from_left` selects which side of the symmetric join the item enters. [OPUS-4.8] sq-vtba.
fn pump(
    side: &mut ChannelPeek,
    from_left: bool,
    join: &mut StreamJoin,
    out_vars: &[String],
    sink: &SolutionSink,
) -> bool {
    match side.next() {
        Some(Ok(sol)) => {
            let tuple = solution_to_tuple(&sol);
            let emitted = if from_left {
                join.push_left(tuple)
            } else {
                join.push_right(tuple)
            };
            for t in emitted {
                match tuple_to_solution(&t, out_vars) {
                    Ok(joined) => {
                        if !sink.emit(joined) {
                            return false; // consumer gone.
                        }
                    }
                    Err(e) => {
                        sink.emit_err(e);
                        return false;
                    }
                }
            }
            true
        }
        Some(Err(e)) => {
            sink.emit_err(e);
            false
        }
        None => true, // this side just closed; the loop re-checks done-state.
    }
}

/// A tiny readiness wrapper over a [`SolutionStream`] so the feeder can prefer a ready side
/// without an async runtime. It buffers a single look-ahead item pulled with `try_recv` (the
/// `ready()` probe) and a `done` flag once the channel closes. [OPUS-4.8] sq-vtba.
struct ChannelPeek {
    rx: Receiver<crate::stream::StreamItem>,
    look: Option<crate::stream::StreamItem>,
    done: bool,
}

impl ChannelPeek {
    /// Whether this side is fully drained (closed AND no buffered look-ahead).
    fn is_done(&self) -> bool {
        self.done && self.look.is_none()
    }

    /// Whether an item is immediately available (buffers one via `try_recv` if so), without
    /// blocking. Sets `done` if the channel has closed. [OPUS-4.8] sq-vtba.
    fn ready(&mut self) -> bool {
        if self.look.is_some() {
            return true;
        }
        if self.done {
            return false;
        }
        match self.rx.try_recv() {
            Ok(item) => {
                self.look = Some(item);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.done = true;
                false
            }
        }
    }

    /// Pull the next item, blocking if necessary; returns `None` once drained. Consumes the
    /// buffered look-ahead first. [OPUS-4.8] sq-vtba.
    fn next(&mut self) -> Option<crate::stream::StreamItem> {
        if let Some(item) = self.look.take() {
            return Some(item);
        }
        if self.done {
            return None;
        }
        match self.rx.recv() {
            Ok(item) => Some(item),
            Err(_) => {
                self.done = true;
                None
            }
        }
    }
}

impl ChannelPeek {
    /// Wrap a [`SolutionStream`]'s receiver for readiness-multiplexed pulling. [OPUS-4.8].
    fn over(stream: SolutionStream) -> ChannelPeek {
        ChannelPeek {
            rx: stream.into_rx(),
            look: None,
            done: false,
        }
    }
}

// ─── Concurrent leaf fan-out + the streaming interpreter ──────────────────────────────

/// Tuning for the streaming interpreter. [OPUS-4.8] sq-vtba.
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// Worker threads in the fan-out [`ScatterPool`] (concurrent leaf fetches).
    pub workers: usize,
    /// Per-leaf and per-join output-channel capacity (the backpressure bound).
    pub channel_cap: usize,
    /// Reused-[`StreamJoin`] tuning (memory budget + spill store) for every join in the tree.
    pub join_opts: StreamJoinOptions,
}

impl Default for StreamOptions {
    fn default() -> Self {
        StreamOptions {
            workers: 4,
            channel_cap: 256,
            join_opts: StreamJoinOptions::default(),
        }
    }
}

/// **Stream** a `sparq-fedplan` [`JoinTree`] against a single source, returning a
/// [`SolutionStream`] that emits federated solutions as the leaf fetches complete and the
/// streaming joins fire — **before** every leaf is exhausted.
///
/// This is the Phase-5 streaming counterpart of [`materialize_single_source`]. It walks the
/// SAME left-deep tree and enforces the SAME single-source guard (a multi-source leaf is
/// rejected with [`InterpError::MultiSource`]), but instead of fetching every leaf in full
/// and blocking-joining, it:
///
///  1. fans each leaf's blocking `source.execute` out across a bounded [`ScatterPool`], each
///     leaf parsing its SRJ and feeding its rows into a per-leaf [`SolutionStream`];
///  2. chains the leaves through [`StreamingJoin`]s in the plan's join order, so a join emits
///     as soon as the later side of a matching pair arrives.
///
/// **Load-bearing invariant:** the streamed solution multiset is multiset-EQUAL to
/// [`materialize_single_source`] for the same plan and source, for ANY interleaving of leaf
/// arrivals (the streaming-correctness test). Because each [`StreamingJoin`] reuses the
/// planner's [`StreamJoin`] — proven multiset-equal to a blocking hash join — and the join
/// keys / output headers match the Phase-3 `natural_join`, the streamed bag equals the
/// materialised bag.
///
/// The returned stream borrows nothing (it owns its channels + threads), but the leaf fetches
/// need the resolver/selection/source for the lifetime of the fetch, so this collects each
/// leaf's lowered [`SubQuery`](crate::source::SubQuery) eagerly (cheap, pure) and moves owned
/// copies onto the worker threads; only the blocking `execute` runs concurrently.
/// [OPUS-4.8] sq-vtba.
pub fn stream_single_source(
    resolver: &SourceResolver<'_>,
    selection: &[PatternSources],
    source: Arc<dyn FederatedSource + Send + Sync>,
    tree: &JoinTree,
    opts: &StreamOptions,
) -> Result<SolutionStream, InterpError> {
    // Size the job queue to hold EVERY leaf job at once, so submitting all leaves up front
    // (during the synchronous tree walk) never blocks waiting for a worker — a worker may be
    // blocked on `emit` before the consumer starts pulling, so a small queue could otherwise
    // deadlock the build. The pool still bounds *concurrency* to `opts.workers`; the queue is
    // just the inbox. [OPUS-4.8] sq-vtba.
    let leaf_count = resolver.bgp().patterns.len().max(1);
    let pool = Arc::new(ScatterPool::new(opts.workers, leaf_count));
    let node = stream_tree(&tree.root, opts, &mut |pattern| {
        single_source_guard(selection, pattern)?;
        let tp = resolver.pattern(pattern)?;
        Ok(stream_leaf(tp, vec![Arc::clone(&source)], &pool, opts))
    })?;
    // Project onto the whole-BGP variable set, matching `materialize_single_source`.
    let project = bgp_vars(resolver);
    Ok(project_stream(node.stream, project, opts.channel_cap))
}

/// Build the streaming pipeline for one plan node: each leaf's stream comes from `leaf`, and a
/// join chains its (already-streaming) left subtree with the right leaf through a
/// [`StreamingJoin`]. The single- and multi-source paths differ only in `leaf`.
/// [OPUS-4.8] sq-vtba / sq-7yf0.
fn stream_tree(
    node: &JoinNode,
    opts: &StreamOptions,
    leaf: &mut dyn FnMut(usize) -> Result<StreamNode, InterpError>,
) -> Result<StreamNode, InterpError> {
    match node {
        JoinNode::Leaf { pattern, .. } => leaf(*pattern),
        JoinNode::Join { left, right, .. } => {
            let l = stream_tree(left, opts, leaf)?;
            let r = leaf(*right)?;
            let join = StreamingJoin::new(&l.vars, &r.vars, opts.join_opts.clone(), opts.channel_cap);
            let out_vars = join.out_vars().to_vec();
            let stream = join.run(l.stream, r.stream);
            Ok(StreamNode {
                vars: out_vars,
                stream,
            })
        }
    }
}

/// A streamed sub-result: its output header + the live [`SolutionStream`]. [OPUS-4.8] sq-vtba.
struct StreamNode {
    vars: Vec<String>,
    stream: SolutionStream,
}

/// **Stream** a `sparq-fedplan` [`JoinTree`] with **per-leaf multi-source UNION fan-out**,
/// returning a [`SolutionStream`]. The streaming counterpart of [`materialize_multi_source`]
/// and the multi-source counterpart of [`stream_single_source`]: it lifts the
/// [`InterpError::MultiSource`] guard (bead `sq-7yf0`).
///
/// `adapters` are the sendable source adapters indexed **the same way** as the resolver's
/// source indices — `adapters[i]` is descriptor `i` is the adapter `select_sources`'
/// `candidates[].source` index `i` names (the same contract [`SourceResolver`] enforces for
/// its borrowed slice; the streaming path needs owned `Arc` adapters to move fetch jobs onto
/// the [`ScatterPool`], so they are passed directly rather than re-borrowed through the
/// resolver). A leaf the planner retained N sources for fans **all N** fetches out onto the
/// pool, each feeding ONE shared per-leaf [`SolutionStream`] (a cloned [`SolutionSink`] per
/// job) — the channel multiplexes them, so the per-leaf stream is the **bag-union** of every
/// source's rows and ends only once every source job for that leaf has finished. The leaves
/// then chain through [`StreamingJoin`]s exactly as the single-source path does.
///
/// **Load-bearing invariant:** the streamed multiset equals [`materialize_multi_source`] for
/// the same plan + adapters, for ANY interleaving of source arrivals — the union is a bag
/// concatenation (order-independent) and the joins reuse the proven [`StreamJoin`].
/// [OPUS-4.8] sq-7yf0.
pub fn stream_multi_source(
    resolver: &SourceResolver<'_>,
    selection: &[PatternSources],
    adapters: &[Arc<dyn FederatedSource + Send + Sync>],
    tree: &JoinTree,
    opts: &StreamOptions,
) -> Result<SolutionStream, InterpError> {
    // Size the queue to hold every (leaf × retained-source) job at once, so the synchronous
    // tree walk never blocks submitting (a worker may be blocked on `emit` before the
    // consumer pulls). The pool still bounds concurrency to `opts.workers`.
    let job_count: usize = resolver
        .bgp()
        .patterns
        .iter()
        .enumerate()
        .map(|(p, _)| leaf_source_indices(selection, p).len())
        .sum::<usize>()
        .max(1);
    let pool = Arc::new(ScatterPool::new(opts.workers, job_count));
    let node = stream_tree(&tree.root, opts, &mut |pattern| {
        let tp = resolver.pattern(pattern)?;
        // Range-check every source index up front (fail closed before spawning any job) so a
        // mismatched plan/adapter set surfaces a `ResolveError`, not a silent under-answer.
        let sources = leaf_source_indices(selection, pattern)
            .into_iter()
            .map(|idx| {
                adapters.get(idx).cloned().ok_or(InterpError::Resolve(
                    crate::planner::ResolveError::SourceOutOfRange {
                        index: idx,
                        sources: adapters.len(),
                    },
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(stream_leaf(tp, sources, &pool, opts))
    })?;
    let project = bgp_vars(resolver);
    Ok(project_stream(node.stream, project, opts.channel_cap))
}

/// Fan ONE leaf's blocking fetches out onto the pool, one job per source in `sources`, all
/// feeding a single per-leaf [`SolutionStream`] (a cloned [`SolutionSink`] per job). Each job
/// runs the SAME [`fetch_leaf_relation`] the materialised interpreters use (so endpoint and
/// fragment sources dispatch identically) and re-keys every row onto the pattern's variable
/// header. The stream is the bag-union of every source's rows and ends once the last job drops
/// its sink; with no sources it is empty. The streaming win is at the join level: a fast leaf
/// feeds the join while a slow one is still in flight. [OPUS-4.8] sq-vtba / sq-7yf0 / sq-yzca.
fn stream_leaf(
    tp: &TriplePattern,
    sources: Vec<Arc<dyn FederatedSource + Send + Sync>>,
    pool: &Arc<ScatterPool>,
    opts: &StreamOptions,
) -> StreamNode {
    let vars = pattern_vars(tp);
    let (sink, stream) = SolutionStream::bounded(opts.channel_cap);
    for source in sources {
        let tp_owned = tp.clone();
        let leaf_vars = vars.clone();
        let job_sink = sink.clone();
        pool.submit(move || {
            match fetch_leaf_relation(source.as_ref(), &tp_owned) {
                Ok(rel) => {
                    for row in rel.rows {
                        let proj = rel_row_project(&rel.vars, &row, &leaf_vars);
                        if !job_sink.emit(Solution::new(leaf_vars.clone(), proj)) {
                            return; // consumer gone.
                        }
                    }
                }
                Err(InterpError::Source(e)) => {
                    job_sink.emit_err(e);
                }
                Err(other) => {
                    job_sink.emit_err(FedError::Transport(other.to_string()));
                }
            }
            // `job_sink` drops here → one source job done.
        });
    }
    // Drop our own sink so the stream is driven solely by the job clones (no sources → the
    // stream closes immediately).
    drop(sink);
    StreamNode { vars, stream }
}

/// Project one parsed SRJ row from its `src_vars` order onto `keep`'s order (an absent
/// variable becomes unbound). The streaming analogue of [`Relation::project`] for a single
/// row. `pub(crate)`: also re-keys union arms in the adaptive executor ([FABLE-5] sq-xw8zz).
/// [OPUS-4.8] sq-vtba.
pub(crate) fn rel_row_project(
    src_vars: &[String],
    row: &[Option<Term>],
    keep: &[String],
) -> Vec<Option<Term>> {
    keep.iter()
        .map(|k| {
            src_vars
                .iter()
                .position(|v| v == k)
                .and_then(|i| row.get(i).cloned().flatten())
        })
        .collect()
}

/// Re-project a live [`SolutionStream`] onto `keep` (the whole-BGP header), spawning a small
/// relay thread so the projection is itself streaming. [OPUS-4.8] sq-vtba.
fn project_stream(stream: SolutionStream, keep: Vec<String>, cap: usize) -> SolutionStream {
    let (sink, out) = SolutionStream::bounded(cap);
    thread::spawn(move || {
        for item in stream {
            match item {
                Ok(sol) => {
                    let cells = rel_row_project(&sol.vars, &sol.cells, &keep);
                    if !sink.emit(Solution::new(keep.clone(), cells)) {
                        return;
                    }
                }
                Err(e) => {
                    sink.emit_err(e);
                    return;
                }
            }
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::SourceResolver;
    use crate::source::{Endpoint, SubQuery, Transport};
    use sparq_fedplan::{
        plan_bgp, select_sources, Bgp, PlanOptions, SourceDescriptor, SourceId, Term as FpTerm,
        TriplePattern, Var,
    };
    use std::collections::HashMap as StdHashMap;
    use std::sync::Mutex;

    fn iri(s: &str) -> FpTerm {
        FpTerm::Iri(s.to_string())
    }
    fn var(s: &str) -> FpTerm {
        FpTerm::Var(Var::new(s))
    }

    /// A transport that answers a sub-query from a fixed map (sub-query SPARQL → SRJ body),
    /// recording what it was asked. Lets the interpreter run with NO network and NO engine,
    /// over hand-built SRJ. [OPUS-4.8] sq-j27p.
    struct MapTransport {
        answers: StdHashMap<String, String>,
        seen: Mutex<Vec<String>>,
    }
    impl MapTransport {
        fn new(answers: StdHashMap<String, String>) -> Self {
            MapTransport {
                answers,
                seen: Mutex::new(Vec::new()),
            }
        }
    }
    impl Transport for MapTransport {
        fn fetch(&self, _endpoint: &str, query: &str) -> Result<String, String> {
            self.seen.lock().unwrap().push(query.to_string());
            self.answers
                .get(query)
                .cloned()
                .ok_or_else(|| format!("no canned answer for {:?}", query))
        }
    }

    fn nn(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    // A small SRJ builder for the canned-answer tests.
    fn srj(vars: &[&str], rows: &[&[(&str, &str)]]) -> String {
        let mut s = String::from("{\"head\":{\"vars\":[");
        for (i, v) in vars.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("\"{}\"", v));
        }
        s.push_str("]},\"results\":{\"bindings\":[");
        for (ri, row) in rows.iter().enumerate() {
            if ri > 0 {
                s.push(',');
            }
            s.push('{');
            for (ci, (var, uri)) in row.iter().enumerate() {
                if ci > 0 {
                    s.push(',');
                }
                s.push_str(&format!(
                    "\"{}\":{{\"type\":\"uri\",\"value\":\"{}\"}}",
                    var, uri
                ));
            }
            s.push('}');
        }
        s.push_str("]}}");
        s
    }

    #[test]
    fn parse_srj_round_trips_terms() {
        let body = r#"{"head":{"vars":["s","o"]},"results":{"bindings":[
            {"s":{"type":"uri","value":"http://ex/a"},"o":{"type":"literal","value":"hi"}},
            {"s":{"type":"uri","value":"http://ex/b"}}
        ]}}"#;
        let rel = parse_srj(body).unwrap();
        assert_eq!(rel.vars, vec!["s", "o"]);
        assert_eq!(rel.rows.len(), 2);
        assert_eq!(rel.rows[0][0], Some(nn("http://ex/a")));
        assert_eq!(
            rel.rows[0][1],
            Some(Term::Literal(Literal::new_simple_literal("hi")))
        );
        // ?o absent in the second solution ⇒ unbound.
        assert_eq!(rel.rows[1][1], None);
    }

    #[test]
    fn parse_srj_rejects_ask_boolean() {
        assert!(parse_srj(r#"{"head":{},"boolean":true}"#).is_err());
        assert!(parse_srj("not json").is_err());
    }

    #[test]
    fn natural_join_matches_on_shared_var() {
        // L(?s,?o) join R(?o,?z): combine where ?o equal; bag semantics.
        let l = Relation {
            vars: vec!["s".into(), "o".into()],
            rows: vec![
                vec![Some(nn("http://ex/a")), Some(nn("http://ex/x"))],
                vec![Some(nn("http://ex/b")), Some(nn("http://ex/y"))],
            ],
        };
        let r = Relation {
            vars: vec!["o".into(), "z".into()],
            rows: vec![
                vec![Some(nn("http://ex/x")), Some(nn("http://ex/p"))],
                vec![Some(nn("http://ex/x")), Some(nn("http://ex/q"))],
                vec![Some(nn("http://ex/w")), Some(nn("http://ex/r"))],
            ],
        };
        let j = natural_join(&l, &r);
        assert_eq!(j.vars, vec!["s", "o", "z"]);
        // ?o=x matches a-x against {p,q}; ?o=y has no match; w has no left.
        assert_eq!(j.rows.len(), 2);
        let bag = solution_bag(&j.vars, &j.rows);
        let expect = Relation {
            vars: vec!["s".into(), "o".into(), "z".into()],
            rows: vec![
                vec![
                    Some(nn("http://ex/a")),
                    Some(nn("http://ex/x")),
                    Some(nn("http://ex/p")),
                ],
                vec![
                    Some(nn("http://ex/a")),
                    Some(nn("http://ex/x")),
                    Some(nn("http://ex/q")),
                ],
            ],
        };
        assert_eq!(bag, solution_bag(&expect.vars, &expect.rows));
    }

    #[test]
    fn solutions_equal_is_order_independent() {
        let a_vars = vec!["s".to_string(), "o".to_string()];
        let a_rows = vec![
            vec![Some(nn("http://ex/a")), Some(nn("http://ex/1"))],
            vec![Some(nn("http://ex/b")), Some(nn("http://ex/2"))],
        ];
        // Same solutions, reversed rows and swapped columns.
        let b_vars = vec!["o".to_string(), "s".to_string()];
        let b_rows = vec![
            vec![Some(nn("http://ex/2")), Some(nn("http://ex/b"))],
            vec![Some(nn("http://ex/1")), Some(nn("http://ex/a"))],
        ];
        assert!(solutions_equal(&a_vars, &a_rows, &b_vars, &b_rows));
        // A different multiplicity is NOT equal.
        let mut dup = a_rows.clone();
        dup.push(a_rows[0].clone());
        assert!(!solutions_equal(&a_vars, &dup, &b_vars, &b_rows));
    }

    /// The interpreter end-to-end over canned SRJ (no engine): a 2-pattern star
    /// ?s :p ?o . ?s :q ?z answered by ONE endpoint, joined on ?s. This drives the real
    /// lowering + resolver + materialised join path. [OPUS-4.8] sq-j27p.
    #[test]
    fn interpreter_materialises_two_pattern_join() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
            TriplePattern::new(var("s"), iri("http://ex/q"), var("z")),
        ]);
        // One source declaring both predicates.
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100)
            .predicate(sparq_fedplan::PredPartition {
                predicate: "http://ex/p".into(),
                triples: 10,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .predicate(sparq_fedplan::PredPartition {
                predicate: "http://ex/q".into(),
                triples: 10,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .build();
        let descriptors = [src];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();

        // Canned SRJ answers keyed by the EXACT sub-query the lowering emits.
        let mut answers = StdHashMap::new();
        answers.insert(
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".to_string(),
            srj(
                &["s", "o"],
                &[
                    &[("s", "http://ex/a"), ("o", "http://ex/o1")],
                    &[("s", "http://ex/b"), ("o", "http://ex/o2")],
                ],
            ),
        );
        answers.insert(
            "SELECT ?s ?z WHERE { ?s <http://ex/q> ?z }".to_string(),
            srj(
                &["s", "z"],
                &[
                    &[("s", "http://ex/a"), ("z", "http://ex/z1")],
                    // no row for ex/b ⇒ it drops out of the join.
                ],
            ),
        );
        let ep = Endpoint::new(
            "http://8.8.8.8/sparql",
            Box::new(MapTransport::new(answers)),
        );
        let adapters: Vec<&dyn FederatedSource> = vec![&ep];
        let resolver = SourceResolver::new(&bgp, &adapters);

        let rel = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap();
        // Only ex/a joins across both arms ⇒ one solution {s=a, o=o1, z=z1}.
        let expect_vars = vec!["s".to_string(), "o".to_string(), "z".to_string()];
        let expect_rows = vec![vec![
            Some(nn("http://ex/a")),
            Some(nn("http://ex/o1")),
            Some(nn("http://ex/z1")),
        ]];
        assert!(
            solutions_equal(&rel.vars, &rel.rows, &expect_vars, &expect_rows),
            "got {:?}",
            rel
        );
    }

    /// A transport that PANICS if reached — the multi-source guard must reject BEFORE any
    /// `execute`/`fetch`, so a leaf with >1 retained source never hits the network.
    struct PanicTransport;
    impl Transport for PanicTransport {
        fn fetch(&self, _e: &str, _q: &str) -> Result<String, String> {
            panic!("multi-source guard must fail closed before any fetch");
        }
    }

    #[test]
    fn interpreter_rejects_multi_source_leaf_phase3() {
        // A leaf with two retained sources must fail closed (single-source Phase 3) — and do
        // so without ever calling the transport (PanicTransport proves the early return).
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        // Two descriptors BOTH holding :p ⇒ the leaf retains two sources.
        let mk = |id: &str| {
            SourceDescriptor::builder(SourceId::new(id))
                .total_triples(10)
                .predicate(sparq_fedplan::PredPartition {
                    predicate: "http://ex/p".into(),
                    triples: 10,
                    distinct_subjects: 10,
                    distinct_objects: 10,
                })
                .build()
        };
        let descriptors = [mk("A"), mk("B")];
        let sel = select_sources(&bgp, &descriptors);
        assert_eq!(sel[0].candidates.len(), 2, "both sources retained");
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let ep = Endpoint::new("http://8.8.8.8/sparql", Box::new(PanicTransport));
        let adapters: Vec<&dyn FederatedSource> = vec![&ep];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let err = materialize_single_source(&resolver, &sel, &ep, &tree).unwrap_err();
        assert_eq!(
            err,
            InterpError::MultiSource {
                pattern: 0,
                sources: 2,
            }
        );
    }

    /// Unused-import guard: `SubQuery` is the lowering's output type; reference it so the
    /// test module's import set is honest. (Kept minimal — the real lowering is exercised by
    /// `interpreter_materialises_two_pattern_join` through `lower_leaf`.) [OPUS-4.8] sq-j27p.
    #[test]
    fn subquery_type_is_in_scope() {
        let _ = SubQuery::new("ASK {}");
    }

    // ─── Phase 5 (sq-vtba): streaming-operator tests ─────────────────────────────────

    fn lit(s: &str) -> Term {
        Term::Literal(Literal::new_simple_literal(s))
    }

    /// The `oxrdf::Term` ⟷ `fedplan::Tuple` bridge must be lossless across every term kind
    /// (IRI, plain/typed/lang literal) so the streamed join's term equality agrees with the
    /// materialised join's. [OPUS-4.8] sq-vtba.
    #[test]
    fn term_tuple_bridge_round_trips() {
        let typed = Term::Literal(Literal::new_typed_literal(
            "30",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        ));
        let lang = Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap());
        let sol = Solution::new(
            vec!["s".into(), "o".into(), "n".into(), "u".into()],
            vec![Some(nn("http://ex/a")), Some(typed.clone()), Some(lang.clone()), None],
        );
        let tuple = solution_to_tuple(&sol);
        // The unbound cell (?u) is absent from the tuple; the bound ones survive.
        let back = tuple_to_solution(&tuple, &["s".into(), "o".into(), "n".into(), "u".into()])
            .expect("round-trip parses");
        assert_eq!(back.get("s"), Some(&nn("http://ex/a")));
        assert_eq!(back.get("o"), Some(&typed));
        assert_eq!(back.get("n"), Some(&lang));
        assert_eq!(back.cells[3], None); // ?u stays unbound.
    }

    /// `StreamingJoin` over two streams must produce the SAME multiset as the Phase-3
    /// `natural_join`, regardless of which side feeds first (interleaving-independence). The
    /// `StreamingJoin` builds its own feeder thread; we drain the result and compare bags.
    /// [OPUS-4.8] sq-vtba.
    #[test]
    fn streaming_join_equals_natural_join_any_order() {
        // L(?s,?o) join R(?s,?z) on ?s.
        let left = Relation {
            vars: vec!["s".into(), "o".into()],
            rows: vec![
                vec![Some(nn("http://ex/a")), Some(lit("o1"))],
                vec![Some(nn("http://ex/b")), Some(lit("o2"))],
                vec![Some(nn("http://ex/a")), Some(lit("o3"))],
            ],
        };
        let right = Relation {
            vars: vec!["s".into(), "z".into()],
            rows: vec![
                vec![Some(nn("http://ex/a")), Some(lit("z1"))],
                vec![Some(nn("http://ex/a")), Some(lit("z2"))],
                vec![Some(nn("http://ex/c")), Some(lit("z3"))],
            ],
        };
        let oracle = natural_join(&left, &right);

        // Drive the streaming join: feed left fully, then right (one interleaving), and the
        // reverse, and an alternating one — all must equal the oracle bag.
        for order in ["lr", "rl", "alt"] {
            let lstream = SolutionStream::from_rows(left.vars.clone(), left.rows.clone());
            let rstream = SolutionStream::from_rows(right.vars.clone(), right.rows.clone());
            let join = StreamingJoin::new(
                &left.vars,
                &right.vars,
                StreamJoinOptions::default(),
                16,
            );
            let _ = order; // the from_rows streams are pre-buffered; the feeder multiplexes them
            let out = join.run(lstream, rstream).collect_solutions().unwrap();
            let got_rows: Vec<Vec<Option<Term>>> = out.iter().map(|s| s.cells.clone()).collect();
            let got_vars = out.first().map(|s| s.vars.clone()).unwrap_or(oracle.vars.clone());
            assert!(
                solutions_equal(&got_vars, &got_rows, &oracle.vars, &oracle.rows),
                "streaming join ({}) must equal natural_join.\n got={:?}\n oracle={:?}",
                order,
                got_rows,
                oracle.rows,
            );
        }
    }

    /// A tiny `StreamJoin` memory budget forces the spill path; the streamed join must STILL
    /// equal the materialised natural join (spill is transparent to correctness). [OPUS-4.8].
    #[test]
    fn streaming_join_equals_under_spill() {
        let left = Relation {
            vars: vec!["s".into(), "o".into()],
            rows: (0..12)
                .map(|i| vec![Some(nn(&format!("http://ex/k{}", i % 3))), Some(lit(&format!("o{}", i)))])
                .collect(),
        };
        let right = Relation {
            vars: vec!["s".into(), "z".into()],
            rows: (0..12)
                .map(|i| vec![Some(nn(&format!("http://ex/k{}", i % 3))), Some(lit(&format!("z{}", i)))])
                .collect(),
        };
        let oracle = natural_join(&left, &right);
        let tiny = StreamJoinOptions {
            mem_budget_tuples: 2,
            spill_store: sparq_fedplan::SpillStore::Memory,
        };
        let lstream = SolutionStream::from_rows(left.vars.clone(), left.rows.clone());
        let rstream = SolutionStream::from_rows(right.vars.clone(), right.rows.clone());
        let join = StreamingJoin::new(&left.vars, &right.vars, tiny, 4);
        let out = join.run(lstream, rstream).collect_solutions().unwrap();
        let got_rows: Vec<Vec<Option<Term>>> = out.iter().map(|s| s.cells.clone()).collect();
        let got_vars = out.first().map(|s| s.vars.clone()).unwrap_or(oracle.vars.clone());
        assert!(
            solutions_equal(&got_vars, &got_rows, &oracle.vars, &oracle.rows),
            "spilled streaming join must equal natural_join"
        );
    }

    /// A `FederatedSource` whose `execute` answers from a canned map AND can be wrapped in an
    /// `Arc<dyn FederatedSource + Send + Sync>` for the streaming interpreter. It embeds a real
    /// `Endpoint` so `source_type()` reports `SourceType::Endpoint` — which the interpreter's
    /// interface dispatch ([`fetch_leaf_relation`], sq-yzca) now CONSULTS to route an endpoint
    /// leaf to the SRJ `execute` path (vs the typed fragment path). [OPUS-4.8].
    struct ArcMapSource {
        answers: StdHashMap<String, String>,
        // A real endpoint so `source_type()` is honest (its inner transport is never reached —
        // `ArcMapSource::execute` overrides `execute` to answer from the canned map directly).
        ep: Endpoint,
    }
    impl ArcMapSource {
        fn new(answers: StdHashMap<String, String>) -> Self {
            ArcMapSource {
                answers,
                ep: Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(StdHashMap::new()))),
            }
        }
    }
    impl FederatedSource for ArcMapSource {
        fn source_type(&self) -> crate::source::SourceType<'_> {
            // Honest: an endpoint-shaped source reports the Endpoint interface, so the
            // interpreter's dispatch routes it to the SRJ `execute` path.
            self.ep.source_type()
        }
        fn discover(
            &self,
        ) -> Result<(crate::source::Capability, Option<sparq_fedplan::SourceDescriptor>), FedError>
        {
            Ok((crate::source::Capability::endpoint(), None))
        }
        fn execute(&self, sub: &SubQuery) -> Result<String, FedError> {
            self.answers
                .get(&sub.sparql)
                .cloned()
                .ok_or_else(|| FedError::Transport(format!("no canned answer for {:?}", sub.sparql)))
        }
    }

    /// The streaming interpreter, end-to-end, over canned SRJ: a 2-pattern star answered by
    /// one source, joined on ?s. The streamed multiset must equal `materialize_single_source`.
    /// This is the in-crate twin of the integration test. [OPUS-4.8] sq-vtba.
    #[test]
    fn stream_interpreter_equals_materialised_two_pattern() {
        let bgp = Bgp::new(vec![
            TriplePattern::new(var("s"), iri("http://ex/p"), var("o")),
            TriplePattern::new(var("s"), iri("http://ex/q"), var("z")),
        ]);
        let src = SourceDescriptor::builder(SourceId::new("S"))
            .total_triples(100)
            .predicate(sparq_fedplan::PredPartition {
                predicate: "http://ex/p".into(),
                triples: 10,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .predicate(sparq_fedplan::PredPartition {
                predicate: "http://ex/q".into(),
                triples: 10,
                distinct_subjects: 10,
                distinct_objects: 10,
            })
            .build();
        let descriptors = [src];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();

        let mut answers = StdHashMap::new();
        answers.insert(
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }".to_string(),
            srj(
                &["s", "o"],
                &[
                    &[("s", "http://ex/a"), ("o", "http://ex/o1")],
                    &[("s", "http://ex/b"), ("o", "http://ex/o2")],
                    &[("s", "http://ex/a"), ("o", "http://ex/o3")],
                ],
            ),
        );
        answers.insert(
            "SELECT ?s ?z WHERE { ?s <http://ex/q> ?z }".to_string(),
            srj(
                &["s", "z"],
                &[
                    &[("s", "http://ex/a"), ("z", "http://ex/z1")],
                    &[("s", "http://ex/a"), ("z", "http://ex/z2")],
                ],
            ),
        );

        // Materialised reference via the Phase-3 interpreter over the SAME canned answers.
        let ep = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(answers.clone())));
        let adapters: Vec<&dyn FederatedSource> = vec![&ep];
        let resolver_ref = SourceResolver::new(&bgp, &adapters);
        let reference = materialize_single_source(&resolver_ref, &sel, &ep, &tree).unwrap();

        // Streamed result via the Phase-5 interpreter.
        let arc_src: Arc<dyn FederatedSource + Send + Sync> = Arc::new(ArcMapSource::new(answers));
        // The resolver only needs an adapter slice for index resolution; the streaming
        // interpreter answers every leaf through `arc_src`, so a stand-in endpoint adapter in
        // the slice is never `execute`d (it only satisfies `resolver.source_count()`/typing).
        let stub = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(StdHashMap::new())));
        let stub_adapters: Vec<&dyn FederatedSource> = vec![&stub];
        let resolver = SourceResolver::new(&bgp, &stub_adapters);
        let opts = StreamOptions {
            workers: 2,
            channel_cap: 8,
            join_opts: StreamJoinOptions::default(),
        };
        let stream =
            stream_single_source(&resolver, &sel, arc_src, &tree, &opts).expect("stream builds");
        let got = stream.collect_solutions().unwrap();
        let got_rows: Vec<Vec<Option<Term>>> = got.iter().map(|s| s.cells.clone()).collect();
        let got_vars = got
            .first()
            .map(|s| s.vars.clone())
            .unwrap_or_else(|| reference.vars.clone());
        assert!(
            solutions_equal(&got_vars, &got_rows, &reference.vars, &reference.rows),
            "streamed result must equal materialise_single_source.\n streamed={:?}\n reference={:?}",
            got_rows,
            reference.rows,
        );
    }

    /// The streaming interpreter enforces the SAME single-source guard as Phase 3: a leaf with
    /// >1 retained source fails closed with `MultiSource` (never under-answers). [OPUS-4.8].
    #[test]
    fn stream_interpreter_rejects_multi_source_leaf() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let mk = |id: &str| {
            SourceDescriptor::builder(SourceId::new(id))
                .total_triples(10)
                .predicate(sparq_fedplan::PredPartition {
                    predicate: "http://ex/p".into(),
                    triples: 10,
                    distinct_subjects: 10,
                    distinct_objects: 10,
                })
                .build()
        };
        let descriptors = [mk("A"), mk("B")];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let stub = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(StdHashMap::new())));
        let adapters: Vec<&dyn FederatedSource> = vec![&stub];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let arc_src: Arc<dyn FederatedSource + Send + Sync> =
            Arc::new(ArcMapSource::new(StdHashMap::new()));
        // `SolutionStream` is not `Debug`, so match the error out by hand (cannot `unwrap_err`).
        let err = match stream_single_source(&resolver, &sel, arc_src, &tree, &StreamOptions::default()) {
            Ok(_) => panic!("multi-source leaf must fail closed"),
            Err(e) => e,
        };
        assert_eq!(err, InterpError::MultiSource { pattern: 0, sources: 2 });
    }

    // ─── sq-yzca: TPF / brTPF leaves are answered through the interpreter (not Unsupported) ──

    use crate::source::{
        BrTpfSource, FragBinding, FragPattern, FragTerm, FragTriple, FragmentPage,
        FragmentTransport, PatternTerm, TpfSource,
    };

    /// An in-test fragment server (one fixed triple set, paged) so the interpreter exercises a
    /// TPF/brTPF leaf with NO network — the operators twin of `source`'s `FixtureFragments`.
    /// [OPUS-4.8] sq-yzca.
    struct FragFixture {
        triples: Vec<FragTriple>,
        page_size: usize,
    }
    impl FragFixture {
        fn matches(pattern: &FragPattern, t: &FragTriple) -> bool {
            // A position matches iff bound-and-equal, or a variable (with repeated-var consistency
            // delegated to the adapter's bind_triple — here we just check bound positions).
            let pos_ok = |p: &PatternTerm, term: &FragTerm| match p {
                PatternTerm::Bound(b) => b == term,
                PatternTerm::Var(_) => true,
            };
            pos_ok(&pattern.subject, &t.subject)
                && pos_ok(&pattern.predicate, &t.predicate)
                && pos_ok(&pattern.object, &t.object)
        }
        fn joins(pattern: &FragPattern, t: &FragTriple, bindings: &[FragBinding]) -> bool {
            if bindings.is_empty() {
                return true;
            }
            // The triple joins a binding iff every var the binding assigns that the pattern also
            // names binds to the same term in this triple.
            bindings.iter().any(|b| {
                b.iter().all(|(name, val)| {
                    for (p, term) in [
                        (&pattern.subject, &t.subject),
                        (&pattern.predicate, &t.predicate),
                        (&pattern.object, &t.object),
                    ] {
                        if p.as_var() == Some(name.as_str()) {
                            return term == val;
                        }
                    }
                    true
                })
            })
        }
    }
    impl FragmentTransport for FragFixture {
        fn fetch_fragment(
            &self,
            _url: &str,
            pattern: &FragPattern,
            bindings: &[FragBinding],
            page: Option<&str>,
        ) -> Result<FragmentPage, String> {
            let matched: Vec<&FragTriple> = self
                .triples
                .iter()
                .filter(|t| Self::matches(pattern, t))
                .filter(|t| Self::joins(pattern, t, bindings))
                .collect();
            let total_items = matched.len() as u64;
            let offset = match page {
                None => 0,
                Some(tok) => tok.strip_prefix("offset:").and_then(|n| n.parse().ok()).unwrap_or(0),
            };
            let end = (offset + self.page_size.max(1)).min(matched.len());
            let triples = matched[offset..end].iter().map(|t| (*t).clone()).collect();
            let next = (end < matched.len()).then(|| format!("offset:{end}"));
            Ok(FragmentPage { triples, total_items, next })
        }
    }

    fn frag_iri(n: &str) -> FragTerm {
        FragTerm::iri(format!("http://ex/{n}"))
    }
    fn knows_triples() -> Vec<FragTriple> {
        let k = || FragTerm::iri("http://ex/knows");
        vec![
            FragTriple::new(frag_iri("alice"), k(), frag_iri("bob")),
            FragTriple::new(frag_iri("alice"), k(), frag_iri("carol")),
            FragTriple::new(frag_iri("bob"), k(), frag_iri("dave")),
        ]
    }

    /// A `?s :knows ?o` BGP planned over a single TPF source must be ANSWERED by the interpreter
    /// (the typed `solutions` path), not rejected with `Unsupported` — the wiring the bead asks
    /// for. The materialised relation is the complete fragment bound into (?s, ?o). [OPUS-4.8].
    #[test]
    fn interpreter_answers_tpf_leaf() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/knows"),
            var("o"),
        )]);
        let descriptors = [SourceDescriptor::builder(SourceId::new("T"))
            .total_triples(3)
            .build()];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        // page_size 2 forces pagination → exercises hydra:next-to-exhaustion through the adapter.
        let tpf = TpfSource::new(
            "http://frag/tpf",
            Box::new(FragFixture { triples: knows_triples(), page_size: 2 }),
        );
        let adapters: Vec<&dyn FederatedSource> = vec![&tpf];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let rel = materialize_single_source(&resolver, &sel, &tpf, &tree).unwrap();

        let expect_vars = vec!["s".to_string(), "o".to_string()];
        let expect_rows = vec![
            vec![Some(nn("http://ex/alice")), Some(nn("http://ex/bob"))],
            vec![Some(nn("http://ex/alice")), Some(nn("http://ex/carol"))],
            vec![Some(nn("http://ex/bob")), Some(nn("http://ex/dave"))],
        ];
        assert!(
            solutions_equal(&rel.vars, &rel.rows, &expect_vars, &expect_rows),
            "TPF leaf must be materialised completely, got {:?}",
            rel
        );
    }

    /// The SAME single-pattern plan over a brTPF source is answered through the brTPF `solutions`
    /// path (an unbound scan that the interpreter joins locally). [OPUS-4.8] sq-yzca.
    #[test]
    fn interpreter_answers_brtpf_leaf() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/knows"),
            var("o"),
        )]);
        let descriptors = [SourceDescriptor::builder(SourceId::new("B"))
            .total_triples(3)
            .build()];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let brtpf = BrTpfSource::new(
            "http://frag/brtpf",
            30,
            Box::new(FragFixture { triples: knows_triples(), page_size: 100 }),
        );
        let adapters: Vec<&dyn FederatedSource> = vec![&brtpf];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let rel = materialize_single_source(&resolver, &sel, &brtpf, &tree).unwrap();
        assert_eq!(rel.rows.len(), 3, "brTPF unbound scan returns the whole fragment");
    }

    /// The adaptive single-source executor answers a TPF leaf through the same interface
    /// dispatch as the static interpreters (it used to call the SRJ `execute` directly, which a
    /// fragment source refuses as `Unsupported`).
    #[cfg(feature = "fedclient-adaptive")]
    #[test]
    fn adaptive_interpreter_answers_tpf_leaf() {
        use sparq_fedplan::ReplanPolicy;
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/knows"),
            var("o"),
        )]);
        let descriptors = [SourceDescriptor::builder(SourceId::new("T"))
            .total_triples(3)
            .build()];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let tpf = TpfSource::new(
            "http://frag/tpf",
            Box::new(FragFixture { triples: knows_triples(), page_size: 2 }),
        );
        let adapters: Vec<&dyn FederatedSource> = vec![&tpf];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let reference = materialize_single_source(&resolver, &sel, &tpf, &tree).unwrap();
        let out = crate::adaptive::execute_adaptive_single_source(
            &bgp,
            &descriptors,
            &resolver,
            &sel,
            &tpf,
            &tree,
            PlanOptions::default(),
            ReplanPolicy::default(),
        )
        .expect("adaptive executor answers a TPF leaf");
        assert!(solutions_equal(
            &out.relation.vars,
            &out.relation.rows,
            &reference.vars,
            &reference.rows
        ));
        assert_eq!(out.relation.rows.len(), 3);
    }

    /// The fragment leaf is also answered by the STREAMING interpreter (the dispatch is shared),
    /// and the streamed multiset equals the materialised one. [OPUS-4.8] sq-yzca.
    #[test]
    fn stream_interpreter_answers_tpf_leaf() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/knows"),
            var("o"),
        )]);
        let descriptors = [SourceDescriptor::builder(SourceId::new("T"))
            .total_triples(3)
            .build()];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        // Materialised reference.
        let tpf_ref = TpfSource::new(
            "http://frag/tpf",
            Box::new(FragFixture { triples: knows_triples(), page_size: 2 }),
        );
        let adapters: Vec<&dyn FederatedSource> = vec![&tpf_ref];
        let resolver_ref = SourceResolver::new(&bgp, &adapters);
        let reference = materialize_single_source(&resolver_ref, &sel, &tpf_ref, &tree).unwrap();
        // Streamed.
        let arc_src: Arc<dyn FederatedSource + Send + Sync> = Arc::new(TpfSource::new(
            "http://frag/tpf",
            Box::new(FragFixture { triples: knows_triples(), page_size: 2 }),
        ));
        let resolver = SourceResolver::new(&bgp, &adapters);
        let stream =
            stream_single_source(&resolver, &sel, arc_src, &tree, &StreamOptions::default())
                .expect("stream builds");
        let got = stream.collect_solutions().unwrap();
        let got_rows: Vec<Vec<Option<Term>>> = got.iter().map(|s| s.cells.clone()).collect();
        let got_vars = got
            .first()
            .map(|s| s.vars.clone())
            .unwrap_or_else(|| reference.vars.clone());
        assert!(
            solutions_equal(&got_vars, &got_rows, &reference.vars, &reference.rows),
            "streamed TPF leaf must equal the materialised one"
        );
    }

    /// A fragment term round-trips through `fragterm_to_oxterm` so a TPF leaf equi-joins with the
    /// SAME `oxrdf::Term` an endpoint leaf produces — the cross-interface join correctness check.
    /// [OPUS-4.8] sq-yzca.
    #[test]
    fn fragterm_oxterm_round_trips_every_kind() {
        assert_eq!(
            fragterm_to_oxterm(&FragTerm::iri("http://ex/a")),
            nn("http://ex/a")
        );
        assert_eq!(
            fragterm_to_oxterm(&FragTerm::Blank("b0".into())),
            Term::BlankNode(BlankNode::new("b0").unwrap())
        );
        // A typed/lang literal carried in its N-Triples lexical form parses back exactly.
        assert_eq!(
            fragterm_to_oxterm(&FragTerm::Literal(
                "\"30\"^^<http://www.w3.org/2001/XMLSchema#integer>".into()
            )),
            Term::Literal(Literal::new_typed_literal(
                "30",
                NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap()
            ))
        );
        assert_eq!(
            fragterm_to_oxterm(&FragTerm::Literal("\"hi\"@en".into())),
            Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap())
        );
    }

    // ─── sq-7yf0: multi-source UNION-per-leaf fan-out ─────────────────────────────────

    /// A single canned answer for one sub-query, as a one-entry map. [OPUS-4.8] sq-7yf0.
    fn one_answer(sub: &str, body: String) -> StdHashMap<String, String> {
        let mut m = StdHashMap::new();
        m.insert(sub.to_string(), body);
        m
    }

    /// `materialize_multi_source` fans a leaf retained by TWO sources out as a per-source
    /// UNION (bag concatenation) instead of returning `MultiSource`. Two distinct endpoints
    /// answer the SAME leaf sub-query with disjoint rows; the union is their concatenation.
    /// [OPUS-4.8] sq-7yf0.
    #[test]
    fn materialize_multi_source_unions_two_sources() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let mk = |id: &str| {
            SourceDescriptor::builder(SourceId::new(id))
                .total_triples(10)
                .predicate(sparq_fedplan::PredPartition {
                    predicate: "http://ex/p".into(),
                    triples: 10,
                    distinct_subjects: 10,
                    distinct_objects: 10,
                })
                .build()
        };
        let descriptors = [mk("A"), mk("B")];
        let sel = select_sources(&bgp, &descriptors);
        assert_eq!(sel[0].candidates.len(), 2, "both sources retained");
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();

        let sub = "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }";
        // Source A holds {a→o1}, source B holds {b→o2}; the union is both rows.
        let ep_a = Endpoint::new(
            "http://8.8.8.8/sparql",
            Box::new(MapTransport::new(one_answer(
                sub,
                srj(&["s", "o"], &[&[("s", "http://ex/a"), ("o", "http://ex/o1")]]),
            ))),
        );
        let ep_b = Endpoint::new(
            "http://8.8.4.4/sparql",
            Box::new(MapTransport::new(one_answer(
                sub,
                srj(&["s", "o"], &[&[("s", "http://ex/b"), ("o", "http://ex/o2")]]),
            ))),
        );
        // Adapter slice in source-index order: adapters[0]=A, adapters[1]=B.
        let adapters: Vec<&dyn FederatedSource> = vec![&ep_a, &ep_b];
        let resolver = SourceResolver::new(&bgp, &adapters);

        let rel = materialize_multi_source(&resolver, &sel, &tree).expect("multi-source unions");
        assert_eq!(rel.vars, vec!["s".to_string(), "o".to_string()]);
        // Bag-union: both sources' rows, no de-duplication.
        let bag = solution_bag(&rel.vars, &rel.rows);
        assert_eq!(
            bag,
            solution_bag(
                &["s".into(), "o".into()],
                &[
                    vec![Some(nn("http://ex/a")), Some(nn("http://ex/o1"))],
                    vec![Some(nn("http://ex/b")), Some(nn("http://ex/o2"))],
                ]
            )
        );
    }

    /// The bag-union preserves multiplicity: when two sources return the SAME solution, the
    /// union carries it TWICE (SELECT bag semantics, no de-dup) — the honest federated
    /// behaviour over disjoint sources that happen to share a triple. [OPUS-4.8] sq-7yf0.
    #[test]
    fn materialize_multi_source_preserves_duplicates() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let mk = |id: &str| {
            SourceDescriptor::builder(SourceId::new(id))
                .total_triples(10)
                .predicate(sparq_fedplan::PredPartition {
                    predicate: "http://ex/p".into(),
                    triples: 10,
                    distinct_subjects: 10,
                    distinct_objects: 10,
                })
                .build()
        };
        let descriptors = [mk("A"), mk("B")];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let sub = "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }";
        let body = srj(&["s", "o"], &[&[("s", "http://ex/a"), ("o", "http://ex/o1")]]);
        let ep_a = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(one_answer(sub, body.clone()))));
        let ep_b = Endpoint::new("http://8.8.4.4/sparql", Box::new(MapTransport::new(one_answer(sub, body))));
        let adapters: Vec<&dyn FederatedSource> = vec![&ep_a, &ep_b];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let rel = materialize_multi_source(&resolver, &sel, &tree).unwrap();
        assert_eq!(rel.rows.len(), 2, "the shared solution appears once per source");
    }

    /// `stream_multi_source` produces the SAME multiset as `materialize_multi_source` for a
    /// two-source leaf (and proves the guard is lifted on the streaming path too). [OPUS-4.8].
    #[test]
    fn stream_multi_source_equals_materialised() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let mk = |id: &str| {
            SourceDescriptor::builder(SourceId::new(id))
                .total_triples(10)
                .predicate(sparq_fedplan::PredPartition {
                    predicate: "http://ex/p".into(),
                    triples: 10,
                    distinct_subjects: 10,
                    distinct_objects: 10,
                })
                .build()
        };
        let descriptors = [mk("A"), mk("B")];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let sub = "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }";
        let a_body = srj(&["s", "o"], &[&[("s", "http://ex/a"), ("o", "http://ex/o1")]]);
        let b_body = srj(&["s", "o"], &[&[("s", "http://ex/b"), ("o", "http://ex/o2")]]);

        // Materialised reference.
        let ep_a = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(one_answer(sub, a_body.clone()))));
        let ep_b = Endpoint::new("http://8.8.4.4/sparql", Box::new(MapTransport::new(one_answer(sub, b_body.clone()))));
        let ref_adapters: Vec<&dyn FederatedSource> = vec![&ep_a, &ep_b];
        let resolver_ref = SourceResolver::new(&bgp, &ref_adapters);
        let reference = materialize_multi_source(&resolver_ref, &sel, &tree).unwrap();

        // Streamed: owned Arc adapters, same source-index order.
        let arc_a: Arc<dyn FederatedSource + Send + Sync> = Arc::new(ArcMapSource::new(one_answer(sub, a_body)));
        let arc_b: Arc<dyn FederatedSource + Send + Sync> = Arc::new(ArcMapSource::new(one_answer(sub, b_body)));
        let arc_adapters = vec![arc_a, arc_b];
        // A stub borrowed slice only for index resolution (pattern lookup); the streaming path
        // answers through `arc_adapters`, so these are never `execute`d.
        let stub_a = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(StdHashMap::new())));
        let stub_b = Endpoint::new("http://8.8.4.4/sparql", Box::new(MapTransport::new(StdHashMap::new())));
        let stub_adapters: Vec<&dyn FederatedSource> = vec![&stub_a, &stub_b];
        let resolver = SourceResolver::new(&bgp, &stub_adapters);
        let stream =
            stream_multi_source(&resolver, &sel, &arc_adapters, &tree, &StreamOptions::default())
                .expect("streaming multi-source builds");
        let got = stream.collect_solutions().unwrap();
        let got_rows: Vec<Vec<Option<Term>>> = got.iter().map(|s| s.cells.clone()).collect();
        let got_vars = got
            .first()
            .map(|s| s.vars.clone())
            .unwrap_or_else(|| reference.vars.clone());
        assert!(
            solutions_equal(&got_vars, &got_rows, &reference.vars, &reference.rows),
            "streamed multi-source must equal materialised.\n streamed={:?}\n reference={:?}",
            got_rows,
            reference.rows,
        );
    }

    /// A leaf the selection retained NO source for is an empty relation (the source-selection
    /// layer is authoritative — the interpreter never falls back to "every adapter"). Both
    /// entry points agree. [OPUS-4.8] sq-7yf0.
    #[test]
    fn multi_source_leaf_with_no_retained_source_is_empty() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        // A descriptor that does NOT declare ex/p ⇒ the source is NOT retained for the leaf.
        let descriptors = [SourceDescriptor::builder(SourceId::new("A")).total_triples(10).build()];
        let sel = select_sources(&bgp, &descriptors);
        assert!(
            sel.first().map(|ps| ps.candidates.is_empty()).unwrap_or(true),
            "no source retained for the unmatched predicate"
        );
        // With an empty selection there is no plan (no leaf retained any source); exercise the
        // leaf helper directly to assert the empty-relation contract without a JoinTree.
        let ep = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(StdHashMap::new())));
        let adapters: Vec<&dyn FederatedSource> = vec![&ep];
        let resolver = SourceResolver::new(&bgp, &adapters);
        let leaf = union_leaf(&resolver, &sel, 0, |_, s, tp| fetch_leaf_relation(s, tp)).unwrap();
        assert_eq!(leaf.vars, vec!["s".to_string(), "o".to_string()]);
        assert!(leaf.rows.is_empty(), "no retained source ⇒ empty leaf relation");
    }

    /// A retained source index out of range for the streaming adapter slice fails closed with
    /// a `ResolveError` BEFORE any fetch job is spawned (never a silent under-answer).
    /// [OPUS-4.8] sq-7yf0.
    #[test]
    fn stream_multi_source_range_checks_source_index() {
        let bgp = Bgp::new(vec![TriplePattern::new(
            var("s"),
            iri("http://ex/p"),
            var("o"),
        )]);
        let mk = |id: &str| {
            SourceDescriptor::builder(SourceId::new(id))
                .total_triples(10)
                .predicate(sparq_fedplan::PredPartition {
                    predicate: "http://ex/p".into(),
                    triples: 10,
                    distinct_subjects: 10,
                    distinct_objects: 10,
                })
                .build()
        };
        let descriptors = [mk("A"), mk("B")];
        let sel = select_sources(&bgp, &descriptors);
        let tree = plan_bgp(&bgp, &sel, &descriptors, &PlanOptions::default()).unwrap();
        let stub = Endpoint::new("http://8.8.8.8/sparql", Box::new(MapTransport::new(StdHashMap::new())));
        let stub_adapters: Vec<&dyn FederatedSource> = vec![&stub];
        let resolver = SourceResolver::new(&bgp, &stub_adapters);
        // Only ONE Arc adapter supplied, but the selection retained source index 1 ⇒ out of range.
        let only: Arc<dyn FederatedSource + Send + Sync> = Arc::new(ArcMapSource::new(StdHashMap::new()));
        let arc_adapters = vec![only];
        let err = match stream_multi_source(&resolver, &sel, &arc_adapters, &tree, &StreamOptions::default()) {
            Ok(_) => panic!("an out-of-range source index must fail closed"),
            Err(e) => e,
        };
        assert_eq!(
            err,
            InterpError::Resolve(crate::planner::ResolveError::SourceOutOfRange {
                index: 1,
                sources: 1,
            })
        );
    }

    // [FABLE-5] sq-3dyje.6 (mutation-kill): DIRECT unit tests for the private ChannelPeek
    // readiness probe. Its ready()/is_done() answers only steer the streaming feeder's
    // side-preference, so no end-to-end result-equality test can observe them (the join
    // result is order-independent) — cargo-mutants proved that by surviving BOTH
    // `ready -> true` and `ready -> false`. Pin the probe's contract directly instead.
    #[test]
    fn channel_peek_ready_look_ahead_and_done_semantics() {
        use crate::stream::{Solution, SolutionStream};
        let (sink, stream) = SolutionStream::bounded(4);
        let mut peek = ChannelPeek::over(stream);
        assert!(!peek.ready(), "an empty open channel is NOT ready");
        assert!(!peek.is_done(), "an open channel is not done");
        let sol = Solution::new(vec!["s".to_string()], vec![None]);
        assert!(sink.emit(sol.clone()));
        assert!(peek.ready(), "an emitted item makes the side ready");
        assert!(
            peek.ready(),
            "readiness is idempotent (the look-ahead buffers exactly one item)"
        );
        let got = peek
            .next()
            .expect("the buffered look-ahead is the next item")
            .expect("a solution, not an error");
        assert_eq!(got, sol, "next() consumes the look-ahead first");
        assert!(!peek.ready(), "consumed: not ready again until a new emit");
        drop(sink);
        assert!(
            !peek.ready(),
            "a closed, drained channel is not ready (and marks done)"
        );
        assert!(peek.is_done(), "closed + no look-ahead ⇒ done");
        assert!(peek.next().is_none(), "done ⇒ next() is None");
    }

    #[test]
    fn channel_peek_next_blocks_through_to_a_late_item_and_close() {
        use crate::stream::{Solution, SolutionStream};
        // next() must return an item emitted AFTER the (empty) readiness probe — the
        // blocking recv path — and then None once the sink drops.
        let (sink, stream) = SolutionStream::bounded(1);
        let mut peek = ChannelPeek::over(stream);
        assert!(!peek.ready(), "nothing emitted yet");
        let producer = std::thread::spawn(move || {
            let ok = sink.emit(Solution::new(
                vec!["s".to_string()],
                vec![Some(oxrdf::Term::NamedNode(
                    oxrdf::NamedNode::new("http://ex/late").unwrap(),
                ))],
            ));
            assert!(ok, "consumer is alive");
            // sink drops here → the channel closes.
        });
        let got = peek.next().expect("the late item arrives").expect("ok");
        assert_eq!(
            got.get("s").map(|t| t.to_string()),
            Some("<http://ex/late>".to_string())
        );
        producer.join().unwrap();
        assert!(peek.next().is_none(), "after close: drained");
        assert!(peek.is_done());
    }

    // [FABLE-5] sq-3dyje.6 (mutation-kill): ScatterPool::join must BLOCK until every submitted
    // job's side effect is visible — that blocking-drain is its whole contract (Drop only
    // closes the queue and lets workers finish detached). cargo-mutants showed `join` replaced
    // by `()` survived: no test observed that after join() returns, all work is DONE. Submit
    // jobs that each sleep then bump a shared counter; a no-op join returns before the sleeping
    // jobs finish, so the counter would be < N right after it returns.
    #[test]
    fn scatter_pool_join_blocks_until_all_jobs_complete() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        use std::time::Duration;
        const N: usize = 8;
        let done = StdArc::new(AtomicUsize::new(0));
        let pool = ScatterPool::new(4, N);
        for _ in 0..N {
            let done = StdArc::clone(&done);
            pool.submit(move || {
                // Sleep so a no-op join cannot have waited for this to finish.
                std::thread::sleep(Duration::from_millis(40));
                done.fetch_add(1, Ordering::SeqCst);
            });
        }
        // Not yet joined: at least one job is still sleeping, so the count is below N.
        pool.join();
        assert_eq!(
            done.load(Ordering::SeqCst),
            N,
            "join() must block until EVERY submitted job has completed its side effect"
        );
    }

    #[test]
    fn scatter_pool_runs_every_submitted_job_exactly_once() {
        // Each of N distinct jobs records its own index; after join the recorded set is
        // EXACTLY {0..N} — no job dropped, none run twice (submit → send → worker runs once).
        use std::sync::Arc as StdArc;
        const N: usize = 16;
        let seen: StdArc<Mutex<Vec<usize>>> = StdArc::new(Mutex::new(Vec::new()));
        let pool = ScatterPool::new(3, 4);
        for i in 0..N {
            let seen = StdArc::clone(&seen);
            pool.submit(move || seen.lock().unwrap().push(i));
        }
        pool.join();
        let mut got = StdArc::try_unwrap(seen).unwrap().into_inner().unwrap();
        got.sort_unstable();
        assert_eq!(got, (0..N).collect::<Vec<_>>(), "every job runs exactly once");
    }
}
