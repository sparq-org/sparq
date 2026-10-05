//! RDFS forward-chaining materialization.
//!
//! Implements the *useful, non-explosive* RDFS entailment rules (RDF 1.1 Semantics §9.2.1)
//! as a fixpoint over dictionary-encoded triples:
//!
//! | rule | premise | conclusion |
//! |------|---------|------------|
//! | rdfs2  | `(p domain c)`, `(s p o)`         | `(s type c)` |
//! | rdfs3  | `(p range c)`, `(s p o)`          | `(o type c)` |
//! | rdfs5  | `(p subPropertyOf q)`, `(q subPropertyOf r)` | `(p subPropertyOf r)` |
//! | rdfs7  | `(p subPropertyOf q)`, `(s p o)`  | `(s q o)` |
//! | rdfs9  | `(c subClassOf d)`, `(s type c)`  | `(s type d)` |
//! | rdfs11 | `(c subClassOf d)`, `(d subClassOf e)` | `(c subClassOf e)` |
//!
//! Deliberately OMITTED: the axiomatic triples and the reflexive/`rdfs:Resource` rules
//! (rdfs4a/4b, rdfs6, rdfs8, rdfs10, rdfs13). They entail that every resource is an
//! `rdfs:Resource` and every class a subclass of itself/`Resource` — true but useless, and
//! they blow up the store by O(terms). This matches the "RDFS" rule set materialized by
//! production engines (GraphDB/RDF4J `rdfs` minus the axiomatic closure).
//!
//! The fixpoint is **naive** (re-derive from the full set each round until stable) — chosen
//! for obvious correctness; RDFS closures converge in a handful of rounds (≈ hierarchy
//! depth + 1). Semi-naive (delta-only) evaluation is a future optimization; materialization
//! is an opt-in build-time step, never on the query hot path.

use crate::Vocab;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

/// Incremental indexes for SEMI-NAIVE RDFS materialization: a new triple joins only against
/// the relevant index (not a full scan), and each rule is fired once per newly-derived fact
/// (the `delta`). Both directions of every join are covered so transitivity (rdfs5/11)
/// closes correctly. Without this the naive fixpoint is O(N³) on deep hierarchies (the O(N²)
/// subclass closure re-derived O(N) rounds).
#[derive(Default)]
pub(crate) struct RdfsIndex {
    sc_super: FxHashMap<Id, Vec<Id>>, // c -> super-classes d (c subClassOf d)
    sc_sub: FxHashMap<Id, Vec<Id>>,   // d -> sub-classes c
    sp_super: FxHashMap<Id, Vec<Id>>, // p -> super-properties q
    sp_sub: FxHashMap<Id, Vec<Id>>,   // q -> sub-properties p
    domain: FxHashMap<Id, Vec<Id>>,   // p -> domain classes
    range: FxHashMap<Id, Vec<Id>>,    // p -> range classes
    type_sub: FxHashMap<Id, Vec<Id>>, // c -> subjects typed c
    po: FxHashMap<Id, Vec<(Id, Id)>>, // predicate -> (subject, object) assertions
}

impl RdfsIndex {
    pub(crate) fn insert(&mut self, [s, p, o]: [Id; 3], v: &Vocab) {
        if p == v.sub_class {
            self.sc_super.entry(s).or_default().push(o);
            self.sc_sub.entry(o).or_default().push(s);
        } else if p == v.sub_prop {
            self.sp_super.entry(s).or_default().push(o);
            self.sp_sub.entry(o).or_default().push(s);
        } else if p == v.domain {
            self.domain.entry(s).or_default().push(o);
        } else if p == v.range {
            self.range.entry(s).or_default().push(o);
        } else if p == v.ty {
            self.type_sub.entry(o).or_default().push(s);
        }
        self.po.entry(p).or_default().push((s, o));
    }

    /// All immediate RDFS consequences of `[s,p,o]` joining against the current index, pushed
    /// into `out`. Each rule appears in BOTH delta directions (this triple as either premise).
    pub(crate) fn derive(&self, [s, p, o]: [Id; 3], v: &Vocab, out: &mut Vec<[Id; 3]>) {
        if p == v.sub_class {
            // rdfs11 (s sc o)(o sc x)⊢(s sc x) and (c sc s)(s sc o)⊢(c sc o)
            if let Some(xs) = self.sc_super.get(&o) {
                out.extend(xs.iter().map(|&x| [s, v.sub_class, x]));
            }
            if let Some(cs) = self.sc_sub.get(&s) {
                out.extend(cs.iter().map(|&c| [c, v.sub_class, o]));
            }
            // rdfs9 (y type s)(s sc o)⊢(y type o)
            if let Some(ys) = self.type_sub.get(&s) {
                out.extend(ys.iter().map(|&y| [y, v.ty, o]));
            }
        } else if p == v.sub_prop {
            // rdfs5 transitivity (both directions)
            if let Some(xs) = self.sp_super.get(&o) {
                out.extend(xs.iter().map(|&x| [s, v.sub_prop, x]));
            }
            if let Some(cs) = self.sp_sub.get(&s) {
                out.extend(cs.iter().map(|&c| [c, v.sub_prop, o]));
            }
            // rdfs7 (x s y)(s sp o)⊢(x o y)
            if let Some(pairs) = self.po.get(&s) {
                out.extend(pairs.iter().map(|&(x, y)| [x, o, y]));
            }
        } else if p == v.ty {
            // rdfs9 (s type o)(o sc d)⊢(s type d)
            if let Some(ds) = self.sc_super.get(&o) {
                out.extend(ds.iter().map(|&d| [s, v.ty, d]));
            }
        } else if p == v.domain {
            // rdfs2 (x s y)(s domain o)⊢(x type o)
            if let Some(pairs) = self.po.get(&s) {
                out.extend(pairs.iter().map(|&(x, _)| [x, v.ty, o]));
            }
        } else if p == v.range {
            // rdfs3 (x s y)(s range o)⊢(y type o)
            if let Some(pairs) = self.po.get(&s) {
                out.extend(pairs.iter().map(|&(_, y)| [y, v.ty, o]));
            }
        }
        // For the OTHER delta direction of rdfs7/2/3: this (s p o) as the data triple.
        if let Some(qs) = self.sp_super.get(&p) {
            out.extend(qs.iter().map(|&q| [s, q, o]));
        }
        if let Some(cs) = self.domain.get(&p) {
            out.extend(cs.iter().map(|&c| [s, v.ty, c]));
        }
        if let Some(cs) = self.range.get(&p) {
            out.extend(cs.iter().map(|&c| [o, v.ty, c]));
        }
    }
}

/// Below this many input triples the ABox sweep runs single-threaded (rayon fan-out is not
/// worth the overhead). Only referenced by the rayon-backed sweep, so it is gated with
/// `parallel` — without the feature (e.g. sparq-solid's `default-features = false`
/// dependency) it would otherwise be dead code and trip `clippy -D warnings`.
/// [OPUS-4.8] sq-xor3: discovered while running the bead's sparq-solid clippy gate.
#[cfg(feature = "parallel")]
const PAR_THRESHOLD: usize = 4096;

/// Transitive closure of a directed relation (`node -> direct successors`): returns
/// `node -> ALL reachable successors`. BFS per source — the schema graph (subClassOf /
/// subPropertyOf) is small relative to the data, even when deep.
pub(crate) fn transitive_closure(direct: &FxHashMap<Id, Vec<Id>>) -> FxHashMap<Id, Vec<Id>> {
    let mut closure: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for_each_reachable(direct, |start, seen| {
        closure.insert(start, seen.iter().copied().collect());
    });
    closure
}

/// The per-source DFS behind [`transitive_closure`]: calls `f(source, reachable)` for every
/// key of `direct`, without materializing the closure map.
pub(crate) fn for_each_reachable(direct: &FxHashMap<Id, Vec<Id>>, mut f: impl FnMut(Id, &FxHashSet<Id>)) {
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut stack: Vec<Id> = Vec::new();
    for (&start, succ0) in direct {
        seen.clear();
        stack.clear();
        stack.extend_from_slice(succ0);
        while let Some(n) = stack.pop() {
            if seen.insert(n) {
                if let Some(succ) = direct.get(&n) {
                    stack.extend(succ.iter().copied());
                }
            }
        }
        f(start, &seen);
    }
}

/// For each property, the full set of typing classes implied by `dr` (its domain or range
/// triples), gathered over the property AND all its super-properties, each closed upward through
/// the subclass closure. So in one pass `(s p o)` yields `(s type c)` / `(o type c)` for every
/// `c` here — no fixpoint over rdfs2/3 ⋈ rdfs7 ⋈ rdfs9 needed.
pub(crate) fn close_dr(
    dr: &FxHashMap<Id, Vec<Id>>,
    sp_closure: &FxHashMap<Id, Vec<Id>>,
    sc_closure: &FxHashMap<Id, Vec<Id>>,
) -> FxHashMap<Id, Vec<Id>> {
    let props: FxHashSet<Id> = dr.keys().chain(sp_closure.keys()).copied().collect();
    let mut out: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for p in props {
        let mut classes: FxHashSet<Id> = FxHashSet::default();
        let supers = sp_closure.get(&p).map(|v| v.as_slice()).unwrap_or(&[]);
        for &q in std::iter::once(&p).chain(supers) {
            if let Some(cs) = dr.get(&q) {
                for &c in cs {
                    classes.insert(c);
                    if let Some(up) = sc_closure.get(&c) {
                        classes.extend(up.iter().copied());
                    }
                }
            }
        }
        if !classes.is_empty() {
            out.insert(p, classes.into_iter().collect());
        }
    }
    out
}

/// The MONOTONE OWL-RL property layer that can be saturated in the single pass: for each
/// property `p`, the full set of `(r, swapped)` pairs reachable from `p` by composing
/// `rdfs:subPropertyOf` (rdfs7), `owl:inverseOf` (prp-inv), `owl:SymmetricProperty`
/// (prp-symp), and `owl:equivalentProperty` (= bidirectional subPropertyOf). `swapped=false`
/// keeps the subject/object orientation; `swapped=true` transposes it. Closed once over the
/// (small, TBox-only) property-orientation graph — the property analogue of `sc_closure`.
///
/// When present this REPLACES the plain `sp_closure` rewrite in [`emit_consequences`]: an
/// asserted `(x p y)` then emits `(x r y)` for every `(r,false)` and `(y r x)` for every
/// `(r,true)`, and the domain/range typing of `r` is applied with that orientation. Every
/// emitted edge is an OWL-RL entailment (each composition step is a valid rule), and the BFS
/// closure is complete, so the result is byte-identical to the fixpoint over this subset.
#[derive(Default)]
pub(crate) struct PropExpand {
    /// `p -> [(r, swapped)]` — derived predicate `r` and whether subject/object are transposed.
    map: FxHashMap<Id, Vec<(Id, bool)>>,
}

/// All RDFS consequences of one asserted triple against the (already fully-closed) schema,
/// optionally extended by the monotone-OWL property-orientation closure `px`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_consequences(
    [s, p, o]: [Id; 3],
    v: &Vocab,
    sc_closure: &FxHashMap<Id, Vec<Id>>,
    sp_closure: &FxHashMap<Id, Vec<Id>>,
    dom_full: &FxHashMap<Id, Vec<Id>>,
    rng_full: &FxHashMap<Id, Vec<Id>>,
    px: Option<&PropExpand>,
    out: &mut Vec<[Id; 3]>,
) {
    if p == v.ty {
        // rdfs9: (s type o), (o sc* d) ⊢ (s type d)
        //
        // [FABLE-5] sq-pbz04.1.1: with the `substrate-join` feature this arm is computed in
        // a BATCH by the shared substrate join kernels (`sweep_type_join` — an object-keyed
        // probe with a uniform fixed-permutation combine), so it is suppressed here to avoid
        // double emission, mirroring the plain rdfs2/3/7 arm below. The `if` arm itself
        // stays so a type assertion NEVER falls through to the PropExpand/plain arms —
        // branch precedence is part of the pinned behaviour.
        #[cfg(not(feature = "substrate-join"))]
        emit_type_rdfs9([s, p, o], v, sc_closure, out);
        #[cfg(feature = "substrate-join")]
        let _ = sc_closure;
    } else if let Some(px) = px.filter(|px| px.map.contains_key(&p)) {
        // Monotone-OWL path: rewrite the predicate AND orient through inverse/symmetric, then
        // type via domain/range of the (oriented) derived predicate.
        //
        // [FABLE-5] sq-pbz04.1.1 DISPOSITION — PERMANENTLY hand-rolled (not a substrate-join
        // candidate): the per-match combine is DATA-DEPENDENT — each matched `(r, swapped)`
        // row selects its own subject/object orientation — and fans out through a SECOND
        // join, the dom/rng typing keyed on the DERIVED predicate `r` (a column that exists
        // only in the first join's output). The shared kernel emits one fixed-layout row per
        // match, so it cannot express this shape without rebuilding the rule structure
        // around it. Full rationale in `substrate_join.rs`; pinned by
        // `tests::prop_expand_inverse_types_through_oriented_domain`.
        for &(r, swapped) in &px.map[&p] {
            let (rs_, ro_) = if swapped { (o, s) } else { (s, o) };
            out.push([rs_, r, ro_]); // rdfs7 / prp-inv / prp-symp / prp-eqp
            if let Some(cs) = dom_full.get(&r) {
                out.extend(cs.iter().map(|&c| [rs_, v.ty, c])); // rdfs2 (+rdfs9)
            }
            if let Some(cs) = rng_full.get(&r) {
                out.extend(cs.iter().map(|&c| [ro_, v.ty, c])); // rdfs3 (+rdfs9)
            }
        }
    } else {
        // No PropExpand active, OR `p` never appears in the property-orientation TBox (no
        // subPropertyOf/inverseOf/symmetric/equivalent edge) — its expansion is the trivial
        // {(p, false)}, i.e. exactly the plain-RDFS emission below. Falling through here (not
        // skipping!) keeps domain/range typing for properties that have ONLY a domain/range
        // declaration: build_prop_expand's all_props never collects those, so an early
        // `px.map.get(&p) → None` return used to silently drop their rdfs2/rdfs3 typing while
        // the full fixpoint path emitted it.
        //
        // [OPUS-4.8] sq-yk6or: with the `substrate-join` feature this plain rdfs2/3/7 branch is
        // computed in a BATCH by the shared substrate join kernels (see `sweep`), so it is
        // suppressed here to avoid double emission. The `is_plain_branch` partition routes
        // exactly these triples to `sweep_predicate_join` — same emitted set, different machinery.
        #[cfg(not(feature = "substrate-join"))]
        emit_plain_rdfs([s, p, o], v, sp_closure, dom_full, rng_full, out);
        // Under `substrate-join` the plain branch is computed in a batch by the substrate sweep,
        // so this arm is empty and `sp_closure` (used ONLY by the plain branch) is unused here.
        #[cfg(feature = "substrate-join")]
        let _ = sp_closure;
    }
}

/// The plain-RDFS predicate join for one asserted `(s, p, o)`: rdfs7 (subPropertyOf rewrite),
/// rdfs2 (domain typing) and rdfs3 (range typing), keyed on the predicate `p`. Factored out of
/// [`emit_consequences`] so the substrate-join path (`sweep` under the `substrate-join` feature)
/// can compute the SAME emission in a batch through the shared [`sparq_substrate::join`] kernels
/// ([OPUS-4.8] sq-yk6or). The two paths emit the identical multiset (asserted by a test).
#[cfg(any(not(feature = "substrate-join"), test))]
#[inline]
fn emit_plain_rdfs(
    [s, p, o]: [Id; 3],
    v: &Vocab,
    sp_closure: &FxHashMap<Id, Vec<Id>>,
    dom_full: &FxHashMap<Id, Vec<Id>>,
    rng_full: &FxHashMap<Id, Vec<Id>>,
    out: &mut Vec<[Id; 3]>,
) {
    // rdfs7: (s p o), (p sp* q) ⊢ (s q o)
    if let Some(qs) = sp_closure.get(&p) {
        out.extend(qs.iter().map(|&q| [s, q, o]));
    }
    // rdfs2 + rdfs9: domain typing through all super-properties, closed up subclass
    if let Some(cs) = dom_full.get(&p) {
        out.extend(cs.iter().map(|&c| [s, v.ty, c]));
    }
    // rdfs3 + rdfs9: range typing
    if let Some(cs) = rng_full.get(&p) {
        out.extend(cs.iter().map(|&c| [o, v.ty, c]));
    }
}

/// The `rdf:type`/rdfs9 subclass-typing join for one asserted `(s, rdf:type, o)`: type `s`
/// with every superclass `d` in the saturated subclass closure of `o`. Factored out of
/// [`emit_consequences`] so the substrate-join path (`sweep_type_join` under the
/// `substrate-join` feature) can compute the SAME emission in a batch through the shared
/// `sparq_substrate::join` kernels ([FABLE-5] sq-pbz04.1.1, mirroring [`emit_plain_rdfs`]).
/// The two paths emit the identical multiset (asserted by
/// `tests::substrate_join_emits_identical_type_branch`).
#[cfg(any(not(feature = "substrate-join"), test))]
#[inline]
fn emit_type_rdfs9(
    [s, _p, o]: [Id; 3],
    v: &Vocab,
    sc_closure: &FxHashMap<Id, Vec<Id>>,
    out: &mut Vec<[Id; 3]>,
) {
    // rdfs9: (s type o), (o sc* d) ⊢ (s type d)
    if let Some(ds) = sc_closure.get(&o) {
        out.extend(ds.iter().map(|&d| [s, v.ty, d]));
    }
}

/// Whether `(s, p, o)` is routed to the plain-RDFS branch of [`emit_consequences`] (NOT the
/// `rdf:type`/rdfs9 branch and NOT the active-`PropExpand` predicate-rewrite branch) — i.e.
/// the branch the substrate-join path computes in a batch. Mirrors `emit_consequences`'s
/// branch selection exactly so routing is partition-preserving. [OPUS-4.8] sq-yk6or.
#[cfg(feature = "substrate-join")]
#[inline]
fn is_plain_branch([_s, p, _o]: [Id; 3], v: &Vocab, px: Option<&PropExpand>) -> bool {
    p != v.ty && !px.is_some_and(|px| px.map.contains_key(&p))
}

/// Run [`emit_consequences`] over every asserted triple. With the `parallel` feature the sweep
/// fans out over rayon (read-only on the schema closures) into per-thread buffers.
#[cfg(all(feature = "parallel", not(feature = "substrate-join")))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sweep(
    triples: &[[Id; 3]],
    v: &Vocab,
    sc: &FxHashMap<Id, Vec<Id>>,
    sp: &FxHashMap<Id, Vec<Id>>,
    dom: &FxHashMap<Id, Vec<Id>>,
    rng: &FxHashMap<Id, Vec<Id>>,
    px: Option<&PropExpand>,
) -> Vec<[Id; 3]> {
    use rayon::prelude::*;
    if triples.len() < PAR_THRESHOLD {
        let mut out = Vec::new();
        for &t in triples {
            emit_consequences(t, v, sc, sp, dom, rng, px, &mut out);
        }
        return out;
    }
    triples
        .par_iter()
        .fold(Vec::new, |mut acc, &t| {
            emit_consequences(t, v, sc, sp, dom, rng, px, &mut acc);
            acc
        })
        .reduce(Vec::new, |mut a, mut b| {
            a.append(&mut b);
            a
        })
}
#[cfg(all(not(feature = "parallel"), not(feature = "substrate-join")))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sweep(
    triples: &[[Id; 3]],
    v: &Vocab,
    sc: &FxHashMap<Id, Vec<Id>>,
    sp: &FxHashMap<Id, Vec<Id>>,
    dom: &FxHashMap<Id, Vec<Id>>,
    rng: &FxHashMap<Id, Vec<Id>>,
    px: Option<&PropExpand>,
) -> Vec<[Id; 3]> {
    let mut out = Vec::new();
    for &t in triples {
        emit_consequences(t, v, sc, sp, dom, rng, px, &mut out);
    }
    out
}

/// [OPUS-4.8] sq-yk6or (epic sq-pbz04, THE SUBSTRATE PAYOFF): the ABox sweep driving the SHARED
/// [`sparq_substrate::join`] hash-join kernels for the plain rdfs2/3/7 predicate join and —
/// [FABLE-5] sq-pbz04.1.1 — the `rdf:type`/rdfs9 subclass-typing join (an object-keyed probe,
/// `substrate_join::sweep_type_join`): the same `build_table` + `probe_emit` body the SPARQL
/// engine drives, with the reasoner supplying its OWN `JoinKeys` + `Budget` monomorphically
/// (NO `Box<dyn>`). Only the active-`PropExpand` predicate-rewrite branch stays on the
/// hand-rolled adjacency path — its per-match combine is data-dependent (orientation swap) and
/// cascades into a second join on the derived predicate; see the disposition in
/// `substrate_join.rs`. The EMITTED SET is identical to the hand-rolled `sweep` (the downstream
/// `dedup_derived` sorts + dedups, so per-triple order is irrelevant — only the multiset
/// matters). Asserted per-branch by `tests::substrate_join_emits_identical_plain_branch` /
/// `tests::substrate_join_emits_identical_type_branch`, and whole-closure by
/// `tests::closure_is_byte_identical_across_join_paths`.
#[cfg(feature = "substrate-join")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn sweep(
    triples: &[[Id; 3]],
    v: &Vocab,
    sc: &FxHashMap<Id, Vec<Id>>,
    sp: &FxHashMap<Id, Vec<Id>>,
    dom: &FxHashMap<Id, Vec<Id>>,
    rng: &FxHashMap<Id, Vec<Id>>,
    px: Option<&PropExpand>,
) -> Vec<[Id; 3]> {
    let mut out = Vec::new();
    // The plain rdfs2/3/7 branch, computed in a batch through the shared substrate join kernels.
    // The plain-branch partition is exactly the triples `emit_consequences` would route to its
    // `else` arm (see `is_plain_branch`), so build/probe over them reproduces that arm's emission.
    let plain: Vec<[Id; 3]> =
        triples.iter().copied().filter(|&t| is_plain_branch(t, v, px)).collect();
    crate::substrate_join::sweep_predicate_join(&plain, v.ty, sp, dom, rng, &mut out);
    // [FABLE-5] sq-pbz04.1.1: the `rdf:type`/rdfs9 batch — the type-assertion partition
    // (exactly the triples `emit_consequences` routes to its `p == v.ty` arm) probed against
    // the subclass closure, object-keyed, through the SAME shared kernels.
    let typed: Vec<[Id; 3]> = triples.iter().copied().filter(|t| t[1] == v.ty).collect();
    crate::substrate_join::sweep_type_join(&typed, sc, &mut out);
    // The one remaining non-batched branch — the active-`PropExpand` predicate rewrite,
    // RETAINED hand-rolled by disposition (see `substrate_join.rs`) — runs on the direct
    // adjacency path: `emit_consequences` suppresses its plain AND type arms under the
    // `substrate-join` feature, so this loop emits ONLY the PropExpand consequences — no
    // double emission of the batches above.
    for &t in triples {
        emit_consequences(t, v, sc, sp, dom, rng, px, &mut out);
    }
    out
}

/// Drop the candidates already asserted, de-duplicate the rest, and return them sorted (so the
/// materialized output is deterministic). With the `parallel` feature the filter + sort run on
/// rayon — a one-shot dedup, unlike the per-round HashSet churn of the old fixpoint.
#[cfg(feature = "parallel")]
fn dedup_derived(emitted: Vec<[Id; 3]>, original: &FxHashSet<[Id; 3]>) -> Vec<[Id; 3]> {
    use rayon::prelude::*;
    if emitted.len() < PAR_THRESHOLD {
        let mut set: FxHashSet<[Id; 3]> = FxHashSet::default();
        for t in emitted {
            if !original.contains(&t) {
                set.insert(t);
            }
        }
        let mut d: Vec<[Id; 3]> = set.into_iter().collect();
        d.sort_unstable();
        return d;
    }
    let mut d: Vec<[Id; 3]> = emitted.into_par_iter().filter(|t| !original.contains(t)).collect();
    d.par_sort_unstable();
    d.dedup();
    d
}
#[cfg(not(feature = "parallel"))]
fn dedup_derived(emitted: Vec<[Id; 3]>, original: &FxHashSet<[Id; 3]>) -> Vec<[Id; 3]> {
    let mut set: FxHashSet<[Id; 3]> = FxHashSet::default();
    for t in emitted {
        if !original.contains(&t) {
            set.insert(t);
        }
    }
    let mut d: Vec<[Id; 3]> = set.into_iter().collect();
    d.sort_unstable();
    d
}

/// The MONOTONE OWL-RL TBox that the single-pass sweep can saturate alongside RDFS, supplied
/// by the OWL materializer when an ontology uses ONLY these (non-recursive) features. Each is
/// fixed by the input (no OWL-RL rule derives `inverseOf` / `SymmetricProperty` / `equivalent*`
/// / `subPropertyOf`-from-nothing), so saturating them once is complete.
#[derive(Default)]
pub(crate) struct MonoOwl {
    /// `owl:equivalentClass` pairs (folded into subClassOf both ways — scm-eqc + cax-eqc).
    pub equiv_class: Vec<(Id, Id)>,
    /// `owl:equivalentProperty` pairs (folded into subPropertyOf both ways — scm-eqp + prp-eqp).
    pub equiv_prop: Vec<(Id, Id)>,
    /// `owl:inverseOf` direct edges, both directions (prp-inv1/2).
    pub inverse: FxHashMap<Id, Vec<Id>>,
    /// `owl:SymmetricProperty` (prp-symp).
    pub symmetric: FxHashSet<Id>,
}

impl MonoOwl {
    fn needs_prop_expand(&self) -> bool {
        !self.inverse.is_empty() || !self.symmetric.is_empty()
    }
}

/// Build the property-orientation closure (see [`PropExpand`]) from the saturated subPropertyOf
/// closure plus the monotone-OWL inverse/symmetric axioms. BFS over `(property, orientation)`
/// nodes; small (TBox-only). The start node `(p,false)` is INCLUDED so the asserted edge's own
/// predicate is rewritten uniformly (and its domain/range typed); the asserted triple itself is
/// dropped later by the dedup against `original`.
fn build_prop_expand(sp_closure: &FxHashMap<Id, Vec<Id>>, m: &MonoOwl) -> PropExpand {
    PropExpand { map: prop_orientation_closure(sp_closure, &m.inverse, &m.symmetric) }
}

/// The `(property, orientation)` BFS behind [`build_prop_expand`], over explicit
/// inverse/symmetric axioms. Shared with the incremental OWL-RL graph so both engines
/// orient properties identically.
pub(crate) fn prop_orientation_closure(
    sp_closure: &FxHashMap<Id, Vec<Id>>,
    inverse: &FxHashMap<Id, Vec<Id>>,
    symmetric: &FxHashSet<Id>,
) -> FxHashMap<Id, Vec<(Id, bool)>> {
    // Every property mentioned anywhere in the property TBox/ABox flows through here; we expand
    // lazily per start property encountered as a key of the union of relevant maps.
    let starts: FxHashSet<Id> = sp_closure
        .keys()
        .chain(inverse.keys())
        .chain(symmetric.iter())
        .copied()
        .collect();
    // We must also expand properties that only appear as subPropertyOf SUPERS or inverse targets;
    // collect those too so a bare asserted `(x p y)` whose p is e.g. only an inverse target still
    // resolves. (Properties never mentioned in the TBox have the trivial {(p,false)} expansion,
    // handled by the `None` fall-through in emit_consequences via sp_closure absence — but with
    // PropExpand active we need an explicit entry, so include all reachable property ids.)
    let mut all_props: FxHashSet<Id> = starts.clone();
    for sup in sp_closure.values() {
        all_props.extend(sup.iter().copied());
    }
    for inv in inverse.values() {
        all_props.extend(inv.iter().copied());
    }
    let mut map: FxHashMap<Id, Vec<(Id, bool)>> = FxHashMap::default();
    for &p in &all_props {
        let mut seen: FxHashSet<(Id, bool)> = FxHashSet::default();
        let mut stack: Vec<(Id, bool)> = vec![(p, false)];
        while let Some((q, or)) = stack.pop() {
            if !seen.insert((q, or)) {
                continue;
            }
            // subPropertyOf (incl. equivalentProperty folded in earlier): same orientation.
            if let Some(sups) = sp_closure.get(&q) {
                for &r in sups {
                    if !seen.contains(&(r, or)) {
                        stack.push((r, or));
                    }
                }
            }
            // inverseOf: flip orientation.
            if let Some(invs) = inverse.get(&q) {
                for &r in invs {
                    if !seen.contains(&(r, !or)) {
                        stack.push((r, !or));
                    }
                }
            }
            // SymmetricProperty: flip orientation, same predicate.
            if symmetric.contains(&q) && !seen.contains(&(q, !or)) {
                stack.push((q, !or));
            }
        }
        map.insert(p, seen.into_iter().collect());
    }
    map
}

/// Expand `triples` in place with the RDFS closure. Returns the number of NEW triples added.
pub fn materialize_rdfs(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    rdfs_closure(dict, triples, false, &MonoOwl::default())
}

/// The RDFS closure, plus — when `emit_dr_closure` — the OWL-RL `scm-dom1/2` / `scm-rng1/2`
/// domain/range closure triples.
///
/// Strategy: SATURATE THE SCHEMA (TBox) once — subClassOf / subPropertyOf transitive closures
/// and the domain/range typing sets closed through them — then do a SINGLE data-parallel sweep
/// of the assertions (ABox). Because the schema is fully closed, every assertion's consequences
/// are independent and need no fixpoint, so the sweep is embarrassingly parallel. Used directly
/// for RDFS, and by the OWL-RL materializer when the ontology uses no OWL-specific features (the
/// OWL closure is then exactly RDFS + scm-dom/rng).
pub(crate) fn rdfs_closure(
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
    emit_dr_closure: bool,
    mono: &MonoOwl,
) -> usize {
    let v = Vocab::intern(dict);
    let original: FxHashSet<[Id; 3]> = triples.iter().copied().collect();

    // 1. Raw schema maps from the input.
    let mut sc: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut sp: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut dom: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut rng: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    for &[s, p, o] in &original {
        if p == v.sub_class {
            sc.entry(s).or_default().push(o);
        } else if p == v.sub_prop {
            sp.entry(s).or_default().push(o);
        } else if p == v.domain {
            dom.entry(s).or_default().push(o);
        } else if p == v.range {
            rng.entry(s).or_default().push(o);
        }
    }
    // Monotone OWL: fold equivalentClass/equivalentProperty into subClassOf/subPropertyOf both
    // ways BEFORE saturation (scm-eqc/scm-eqp). cax-eqc1/2 and prp-eqp1/2 then follow for free
    // from rdfs9/rdfs7 over the added edges, and the closure emits the subClassOf/subPropertyOf
    // triples exactly as the fixpoint's scm-eqc/eqp do.
    for &(a, b) in &mono.equiv_class {
        sc.entry(a).or_default().push(b);
        sc.entry(b).or_default().push(a);
    }
    for &(a, b) in &mono.equiv_prop {
        sp.entry(a).or_default().push(b);
        sp.entry(b).or_default().push(a);
    }

    // 2. Saturate the schema (small, serial).
    let sc_closure = transitive_closure(&sc);
    let sp_closure = transitive_closure(&sp);
    let dom_full = close_dr(&dom, &sp_closure, &sc_closure);
    let rng_full = close_dr(&rng, &sp_closure, &sc_closure);

    // Property-orientation closure for inverseOf/SymmetricProperty (only when needed).
    let prop_expand = if mono.needs_prop_expand() {
        Some(build_prop_expand(&sp_closure, mono))
    } else {
        None
    };

    // 3. Single parallel ABox sweep + the schema-closure triples (rdfs11 / rdfs5).
    let asserted: Vec<[Id; 3]> = original.iter().copied().collect();
    let mut emitted = sweep(
        &asserted,
        &v,
        &sc_closure,
        &sp_closure,
        &dom_full,
        &rng_full,
        prop_expand.as_ref(),
    );
    for (&c, ds) in &sc_closure {
        emitted.extend(ds.iter().map(|&d| [c, v.sub_class, d]));
    }
    for (&p, qs) in &sp_closure {
        emitted.extend(qs.iter().map(|&q| [p, v.sub_prop, q]));
    }
    // OWL-RL scm-dom1/2 + scm-rng1/2: the saturated domain/range sets ARE their closure.
    if emit_dr_closure {
        for (&p, cs) in &dom_full {
            emitted.extend(cs.iter().map(|&c| [p, v.domain, c]));
        }
        for (&p, cs) in &rng_full {
            emitted.extend(cs.iter().map(|&c| [p, v.range, c]));
        }
    }

    // 4. De-duplicate the derived facts, drop those already asserted, sort for determinism.
    let derived = dedup_derived(emitted, &original);

    let added = derived.len();
    triples.clear();
    triples.extend(original_in_order(&original));
    triples.extend(derived);
    // NOT `triples.len() - before`: a caller may pass DUPLICATE input triples
    // (e.g. RDF/XML loaders emit repeated declarations), which the rebuild
    // dedups — the subtraction then underflows. The new-triple count is the
    // derived set's size by construction.
    added
}


/// The original triples, deduplicated but in a deterministic (sorted) order. (We do not have
/// the caller's original Vec here after clearing; sorting gives a stable, reproducible base.)
fn original_in_order(original: &FxHashSet<[Id; 3]>) -> Vec<[Id; 3]> {
    let mut v: Vec<[Id; 3]> = original.iter().copied().collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::vocab::{rdf, rdfs};
    use oxrdf::{NamedNode, Term};

    fn iri(dict: &mut Dict, s: &str) -> Id {
        dict.intern_iri(s)
    }
    fn ex(dict: &mut Dict, local: &str) -> Id {
        dict.intern_iri(&format!("http://ex/{local}"))
    }
    /// Is the triple (by IRI strings) in the materialized set?
    fn has(dict: &Dict, set: &FxHashSet<[Id; 3]>, s: &str, p: &str, o: &str) -> bool {
        let g = |iri: &str| dict.lookup(&Term::NamedNode(NamedNode::new_unchecked(iri.to_string())));
        let (si, pi, oi) = (g(s), g(p), g(o));
        si != 0 && pi != 0 && oi != 0 && set.contains(&[si, pi, oi])
    }

    #[test]
    fn subclass_transitivity_and_type_propagation() {
        // ex:Dog sc ex:Mammal sc ex:Animal ; ex:rex a ex:Dog.  Expect ex:rex a Mammal, Animal.
        let mut dict = Dict::new();
        let (dog, mammal, animal, rex) =
            (ex(&mut dict, "Dog"), ex(&mut dict, "Mammal"), ex(&mut dict, "Animal"), ex(&mut dict, "rex"));
        let (sc, ty) = (iri(&mut dict, rdfs::SUB_CLASS_OF.as_str()), iri(&mut dict, rdf::TYPE.as_str()));
        let mut triples = vec![[dog, sc, mammal], [mammal, sc, animal], [rex, ty, dog]];
        let added = materialize_rdfs(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(has(&dict, &set, "http://ex/rex", rdf::TYPE.as_str(), "http://ex/Mammal"), "rdfs9 one hop");
        assert!(has(&dict, &set, "http://ex/rex", rdf::TYPE.as_str(), "http://ex/Animal"), "rdfs9 transitive");
        assert!(has(&dict, &set, "http://ex/Dog", rdfs::SUB_CLASS_OF.as_str(), "http://ex/Animal"), "rdfs11");
        assert!(added >= 3);
    }

    #[test]
    fn domain_range_and_subproperty() {
        // ex:hasParent sp ex:relatedTo ; ex:hasParent domain ex:Person ; ex:hasParent range ex:Person.
        // ex:a ex:hasParent ex:b.  Expect: a relatedTo b; a type Person; b type Person.
        let mut dict = Dict::new();
        let (hp, rel, person, a, b) = (
            ex(&mut dict, "hasParent"), ex(&mut dict, "relatedTo"), ex(&mut dict, "Person"),
            ex(&mut dict, "a"), ex(&mut dict, "b"),
        );
        let (sp, dom, rng) = (
            iri(&mut dict, rdfs::SUB_PROPERTY_OF.as_str()),
            iri(&mut dict, rdfs::DOMAIN.as_str()),
            iri(&mut dict, rdfs::RANGE.as_str()),
        );
        let mut triples = vec![[hp, sp, rel], [hp, dom, person], [hp, rng, person], [a, hp, b]];
        materialize_rdfs(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(has(&dict, &set, "http://ex/a", "http://ex/relatedTo", "http://ex/b"), "rdfs7");
        assert!(has(&dict, &set, "http://ex/a", rdf::TYPE.as_str(), "http://ex/Person"), "rdfs2 domain");
        assert!(has(&dict, &set, "http://ex/b", rdf::TYPE.as_str(), "http://ex/Person"), "rdfs3 range");
    }

    #[test]
    fn subproperty_domain_interaction() {
        // ex:p sp ex:q ; ex:q domain ex:C ; ex:s ex:p ex:o.  rdfs7 gives (s q o), then rdfs2
        // on q's domain gives (s type C). Tests the rule-interaction the fixpoint must catch.
        let mut dict = Dict::new();
        let (p, q, c, s, o) = (
            ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "C"), ex(&mut dict, "s"), ex(&mut dict, "o"),
        );
        let (sp, dom) = (iri(&mut dict, rdfs::SUB_PROPERTY_OF.as_str()), iri(&mut dict, rdfs::DOMAIN.as_str()));
        let mut triples = vec![[p, sp, q], [q, dom, c], [s, p, o]];
        materialize_rdfs(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(has(&dict, &set, "http://ex/s", "http://ex/q", "http://ex/o"), "rdfs7");
        assert!(has(&dict, &set, "http://ex/s", rdf::TYPE.as_str(), "http://ex/C"), "rdfs7->rdfs2 interaction");
    }

    #[test]
    fn prop_expand_keeps_domain_range_only_properties() {
        // REGRESSION: with PropExpand active (any inverseOf/symmetric axiom in the ontology), a
        // property that has a domain/range but NO property-orientation edge (no subPropertyOf /
        // inverseOf / symmetric / equivalent) was absent from the px map, and emit_consequences
        // dropped its rdfs2/rdfs3 typing entirely — diverging from the full fixpoint path.
        let mut dict = Dict::new();
        let (p, q, r, c, d, a, b) = (
            ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "r"), ex(&mut dict, "C"),
            ex(&mut dict, "D"), ex(&mut dict, "a"), ex(&mut dict, "b"),
        );
        let (dom, rng, ty) = (
            iri(&mut dict, rdfs::DOMAIN.as_str()),
            iri(&mut dict, rdfs::RANGE.as_str()),
            iri(&mut dict, rdf::TYPE.as_str()),
        );
        // q owl:inverseOf r activates PropExpand; p has ONLY domain/range declarations.
        let mono = MonoOwl {
            inverse: [(q, vec![r]), (r, vec![q])].into_iter().collect(),
            ..MonoOwl::default()
        };
        let mut triples = vec![[p, dom, c], [p, rng, d], [a, p, b]];
        rdfs_closure(&mut dict, &mut triples, false, &mono);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[a, ty, c]), "rdfs2 domain typing must survive active PropExpand");
        assert!(set.contains(&[b, ty, d]), "rdfs3 range typing must survive active PropExpand");
    }

    #[test]
    fn idempotent() {
        let mut dict = Dict::new();
        let (dog, animal, rex) = (ex(&mut dict, "Dog"), ex(&mut dict, "Animal"), ex(&mut dict, "rex"));
        let (sc, ty) = (iri(&mut dict, rdfs::SUB_CLASS_OF.as_str()), iri(&mut dict, rdf::TYPE.as_str()));
        let mut triples = vec![[dog, sc, animal], [rex, ty, dog]];
        materialize_rdfs(&mut dict, &mut triples);
        let n = triples.len();
        let added2 = materialize_rdfs(&mut dict, &mut triples);
        assert_eq!(added2, 0, "second materialization adds nothing");
        assert_eq!(triples.len(), n, "idempotent");
    }

    // ---- behavioural tests for the lowest-covered rdfs.rs paths (sq-jd5s) [OPUS-4.8] ----

    /// Sort a closure map's value vecs so reachable-set comparisons are order-independent.
    fn sorted(mut m: FxHashMap<Id, Vec<Id>>) -> FxHashMap<Id, Vec<Id>> {
        for v in m.values_mut() {
            v.sort_unstable();
        }
        m
    }

    #[test]
    fn transitive_closure_chain_diamond_and_cycle() {
        // A 4-node CHAIN 1->2->3->4: 1 reaches {2,3,4}, 2 reaches {3,4}, 3 reaches {4}.
        let chain: FxHashMap<Id, Vec<Id>> =
            [(1, vec![2]), (2, vec![3]), (3, vec![4])].into_iter().collect();
        let c = sorted(transitive_closure(&chain));
        assert_eq!(c[&1], vec![2, 3, 4], "chain head reaches all descendants");
        assert_eq!(c[&2], vec![3, 4]);
        assert_eq!(c[&3], vec![4]);
        // A DIAMOND 1->2, 1->3, 2->4, 3->4: 1 reaches {2,3,4}, and 4 is reached once (deduped).
        let diamond: FxHashMap<Id, Vec<Id>> =
            [(1, vec![2, 3]), (2, vec![4]), (3, vec![4])].into_iter().collect();
        let d = sorted(transitive_closure(&diamond));
        assert_eq!(d[&1], vec![2, 3, 4], "diamond apex reaches all, 4 not duplicated");
        // A CYCLE 1->2->1: BFS over the `seen` set terminates and each node reaches the whole SCC.
        let cycle: FxHashMap<Id, Vec<Id>> = [(1, vec![2]), (2, vec![1])].into_iter().collect();
        let cy = sorted(transitive_closure(&cycle));
        assert_eq!(cy[&1], vec![1, 2], "cycle: node reaches itself + peer (no infinite loop)");
        assert_eq!(cy[&2], vec![1, 2]);
    }

    #[test]
    fn close_dr_inherits_through_superproperty_and_subclass() {
        // p sp q ; q domain D ; D sc C.  close_dr(domain) over p must yield {D, C}: p inherits
        // q's domain (rdfs7-into-rdfs2), and the typing class D is closed UP the subclass graph.
        let sp_closure: FxHashMap<Id, Vec<Id>> = [(1 /*p*/, vec![2 /*q*/])].into_iter().collect();
        let sc_closure: FxHashMap<Id, Vec<Id>> = [(10 /*D*/, vec![11 /*C*/])].into_iter().collect();
        let domain: FxHashMap<Id, Vec<Id>> = [(2 /*q*/, vec![10 /*D*/])].into_iter().collect();
        let full = sorted(close_dr(&domain, &sp_closure, &sc_closure));
        assert_eq!(full[&1], vec![10, 11], "p inherits q's domain D, closed up to C");
        assert_eq!(full[&2], vec![10, 11], "q itself keeps D + its superclass C");
    }

    #[test]
    fn rdfs_index_derive_fires_every_rule_in_both_delta_directions() {
        // Drive RdfsIndex (the semi-naive incremental path used by the OWL fixpoint) directly,
        // asserting each rule fires whether THIS triple is the schema edge or the data edge.
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (a, b, cc, p, q, s, o) = (
            ex(&mut dict, "A"), ex(&mut dict, "B"), ex(&mut dict, "C"),
            ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "s"), ex(&mut dict, "o"),
        );
        let mut idx = RdfsIndex::default();
        // Seed the index with schema + data so both delta directions have a join partner.
        for t in [
            [a, v.sub_class, b],  // A sc B
            [b, v.sub_class, cc], // B sc C
            [p, v.sub_prop, q],   // p sp q
            [s, p, o],            // data: s p o
            [s, v.ty, a],         // data: s type A
        ] {
            idx.insert(t, &v);
        }
        let derive = |idx: &RdfsIndex, t: [Id; 3]| {
            let mut out = Vec::new();
            idx.derive(t, &v, &mut out);
            out
        };
        // rdfs11 forward (A sc B as premise, B sc C in index) ⊢ A sc C.
        assert!(derive(&idx, [a, v.sub_class, b]).contains(&[a, v.sub_class, cc]), "rdfs11 fwd");
        // rdfs11 backward (B sc C as premise, A sc B in index) ⊢ A sc C.
        assert!(derive(&idx, [b, v.sub_class, cc]).contains(&[a, v.sub_class, cc]), "rdfs11 bwd");
        // rdfs9 via subclass edge (A sc B, s type A in index) ⊢ s type B.
        assert!(derive(&idx, [a, v.sub_class, b]).contains(&[s, v.ty, b]), "rdfs9 via sc-edge delta");
        // rdfs9 via type edge (s type A, A sc B in index) ⊢ s type B.
        assert!(derive(&idx, [s, v.ty, a]).contains(&[s, v.ty, b]), "rdfs9 via type-edge delta");
        // rdfs7 via subprop edge (p sp q, s p o in index) ⊢ s q o.
        assert!(derive(&idx, [p, v.sub_prop, q]).contains(&[s, q, o]), "rdfs7 via sp-edge delta");
        // rdfs7 via data edge (s p o, p sp q in index) ⊢ s q o.
        assert!(derive(&idx, [s, p, o]).contains(&[s, q, o]), "rdfs7 via data-edge delta");
        // rdfs2/rdfs3: a data edge whose predicate has domain/range, plus the schema-edge deltas.
        let dprop = ex(&mut dict, "dprop");
        let mut idx2 = RdfsIndex::default();
        for t in [[dprop, v.domain, cc], [dprop, v.range, cc], [s, dprop, o]] {
            idx2.insert(t, &v);
        }
        // data-edge delta: (s dprop o) with dprop having domain+range ⊢ s type C and o type C.
        assert!(derive(&idx2, [s, dprop, o]).contains(&[s, v.ty, cc]), "rdfs2 on data-edge delta");
        assert!(derive(&idx2, [s, dprop, o]).contains(&[o, v.ty, cc]), "rdfs3 on data-edge delta");
        // domain-edge delta: (dprop domain C) joins the existing (s,o) assertion ⊢ s type C.
        assert!(derive(&idx2, [dprop, v.domain, cc]).contains(&[s, v.ty, cc]), "rdfs2 on domain-edge delta");
        // range-edge delta: (dprop range C) ⊢ o type C.
        assert!(derive(&idx2, [dprop, v.range, cc]).contains(&[o, v.ty, cc]), "rdfs3 on range-edge delta");
    }

    #[test]
    fn equivalent_class_and_property_fold_bidirectionally() {
        // MonoOwl.equiv_class folds into subClassOf BOTH ways (scm-eqc): A ≡ B means x type A ⊢
        // x type B. Likewise equiv_prop (scm-eqp): p ≡ q rewrites m p n ⊢ m q n. This mono.equiv_*
        // path is untouched by the plain materialize_rdfs tests.
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (a, b, p, q, x, m, n) = (
            ex(&mut dict, "A"), ex(&mut dict, "B"), ex(&mut dict, "p"), ex(&mut dict, "q"),
            ex(&mut dict, "x"), ex(&mut dict, "m"), ex(&mut dict, "n"),
        );
        let mono = MonoOwl {
            equiv_class: vec![(a, b)],
            equiv_prop: vec![(p, q)],
            ..MonoOwl::default()
        };
        let mut triples = vec![[x, v.ty, a], [m, p, n]];
        rdfs_closure(&mut dict, &mut triples, false, &mono);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[x, v.ty, b]), "equivalentClass: x type A ⊢ x type B");
        assert!(set.contains(&[a, v.sub_class, b]), "equivalentClass emits A sc B");
        assert!(set.contains(&[b, v.sub_class, a]), "equivalentClass emits B sc A (both ways)");
        assert!(set.contains(&[m, q, n]), "equivalentProperty: m p n ⊢ m q n");
        assert!(set.contains(&[p, v.sub_prop, q]), "equivalentProperty emits p sp q");
        assert!(set.contains(&[q, v.sub_prop, p]), "equivalentProperty emits q sp p (both ways)");
    }

    #[test]
    fn symmetric_property_emits_swapped_edge() {
        // owl:SymmetricProperty (prp-symp): m knows n ⊢ n knows m. Drives the PropExpand
        // orientation-flip path (build_prop_expand symmetric branch + emit swapped emission).
        let mut dict = Dict::new();
        let (knows, m, n) = (ex(&mut dict, "knows"), ex(&mut dict, "m"), ex(&mut dict, "n"));
        let mono = MonoOwl { symmetric: [knows].into_iter().collect(), ..MonoOwl::default() };
        let mut triples = vec![[m, knows, n]];
        rdfs_closure(&mut dict, &mut triples, false, &mono);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[n, knows, m]), "prp-symp: symmetric edge is swapped");
    }

    #[test]
    fn inverse_of_emits_swapped_predicate() {
        // owl:inverseOf (prp-inv): hasParent inverse hasChild ; a hasParent b ⊢ b hasChild a.
        let mut dict = Dict::new();
        let (hp, hc, a, b) = (
            ex(&mut dict, "hasParent"), ex(&mut dict, "hasChild"),
            ex(&mut dict, "a"), ex(&mut dict, "b"),
        );
        let mono = MonoOwl {
            inverse: [(hp, vec![hc]), (hc, vec![hp])].into_iter().collect(),
            ..MonoOwl::default()
        };
        let mut triples = vec![[a, hp, b]];
        rdfs_closure(&mut dict, &mut triples, false, &mono);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[b, hc, a]), "prp-inv: inverse predicate with swapped s/o");
    }

    #[test]
    fn emit_dr_closure_emits_scm_dom_rng_triples() {
        // With emit_dr_closure = true (the OWL-RL scm-dom1/2 + scm-rng1/2 path), the saturated
        // domain/range sets are themselves emitted as domain/range triples — and inherited through
        // subPropertyOf. p sp q ; q domain D ⊢ (p domain D) emitted; off by default (RDFS).
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (p, q, d) = (ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "D"));
        let base = vec![[p, v.sub_prop, q], [q, v.domain, d]];
        // Default RDFS: scm-dom NOT emitted.
        let mut off = base.clone();
        rdfs_closure(&mut dict, &mut off, false, &MonoOwl::default());
        let off_set: FxHashSet<[Id; 3]> = off.iter().copied().collect();
        assert!(!off_set.contains(&[p, v.domain, d]), "scm-dom is off in plain RDFS");
        // emit_dr_closure: p inherits q's domain as an explicit (p domain D) triple.
        let mut on = base;
        rdfs_closure(&mut dict, &mut on, true, &MonoOwl::default());
        let on_set: FxHashSet<[Id; 3]> = on.iter().copied().collect();
        assert!(on_set.contains(&[p, v.domain, d]), "scm-dom: subproperty inherits domain triple");
        assert!(on_set.contains(&[q, v.domain, d]), "scm-dom: q's own domain re-emitted");
    }

    #[test]
    fn duplicate_input_triples_do_not_underflow_added_count() {
        // A loader may pass the SAME triple twice. The rebuild dedups, so `added` is the DERIVED
        // set size (not len-after - len-before, which would underflow). Here there is one derivable
        // fact (rdfs9), so added == 1 despite the duplicate input.
        let mut dict = Dict::new();
        let (dog, animal, rex) =
            (ex(&mut dict, "Dog"), ex(&mut dict, "Animal"), ex(&mut dict, "rex"));
        let (sc, ty) =
            (iri(&mut dict, rdfs::SUB_CLASS_OF.as_str()), iri(&mut dict, rdf::TYPE.as_str()));
        let mut triples = vec![[dog, sc, animal], [rex, ty, dog], [rex, ty, dog]];
        let added = materialize_rdfs(&mut dict, &mut triples);
        assert_eq!(added, 1, "only (rex type Animal) is new; duplicate input must not underflow");
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(has(&dict, &set, "http://ex/rex", rdf::TYPE.as_str(), "http://ex/Animal"));
        // The output is deduplicated: the duplicate input appears exactly once.
        let dupes = triples.iter().filter(|&&t| t == [rex, ty, dog]).count();
        assert_eq!(dupes, 1, "duplicate input collapsed in the rebuilt output");
    }

    // ---- [OPUS-4.8] sq-yk6or: the shared-substrate-join adoption is BEHAVIOUR-NEUTRAL ----

    /// The LOAD-BEARING invariant of sq-yk6or: driving the shared `sparq-substrate::join` kernels
    /// for the plain rdfs2/3/7 predicate join emits the BYTE-IDENTICAL multiset as the hand-rolled
    /// `emit_plain_rdfs` adjacency path. Runs both over the SAME (already-saturated) schema and a
    /// rich ABox, sorts each emission, and asserts equality. (Only the substrate path is compiled
    /// under the feature; this test pins the equivalence the conformance ratchet relies on.)
    #[cfg(feature = "substrate-join")]
    #[test]
    fn substrate_join_emits_identical_plain_branch() {
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (p, q, r, c, d, s1, o1, s2, o2) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "q"),
            ex(&mut dict, "r"),
            ex(&mut dict, "C"),
            ex(&mut dict, "D"),
            ex(&mut dict, "s1"),
            ex(&mut dict, "o1"),
            ex(&mut dict, "s2"),
            ex(&mut dict, "o2"),
        );
        // Saturated schema closures (as `rdfs_closure` builds them): p sp* {q, r}; p domain {C};
        // p range {D}; q domain {C} too, so a triple on q types via domain as well.
        let sp: FxHashMap<Id, Vec<Id>> = [(p, vec![q, r])].into_iter().collect();
        let dom: FxHashMap<Id, Vec<Id>> = [(p, vec![c]), (q, vec![c])].into_iter().collect();
        let rng: FxHashMap<Id, Vec<Id>> = [(p, vec![d])].into_iter().collect();
        // A plain-branch ABox: two assertions on p (rewrite + domain + range), one on q (domain
        // only), and one on an unmentioned predicate (emits nothing) — the partition the substrate
        // sweep handles.
        let unknown = ex(&mut dict, "z");
        let abox = vec![[s1, p, o1], [s2, p, o2], [s1, q, o1], [s2, unknown, o2]];

        // Hand-rolled reference emission (the path the default build runs).
        let mut hand = Vec::new();
        for &t in &abox {
            emit_plain_rdfs(t, &v, &sp, &dom, &rng, &mut hand);
        }
        hand.sort_unstable();

        // Shared substrate-join emission.
        let mut shared = Vec::new();
        crate::substrate_join::sweep_predicate_join(&abox, v.ty, &sp, &dom, &rng, &mut shared);
        shared.sort_unstable();

        assert_eq!(
            shared, hand,
            "the shared substrate join must emit the byte-identical plain-RDFS multiset"
        );
        assert!(!hand.is_empty(), "the fixture must actually exercise the join (non-empty emission)");
    }

    /// End-to-end: `materialize_rdfs` under the `substrate-join` feature produces the SAME closure
    /// the hand-rolled path documents in the other rdfs.rs tests — the full pipeline (schema
    /// saturation + substrate sweep + dedup) is behaviour-neutral. Re-checks the canonical
    /// rdfs2/3/7/9/11 derivations on the shared-join path.
    #[cfg(feature = "substrate-join")]
    #[test]
    fn materialize_rdfs_substrate_path_matches_documented_closure() {
        let mut dict = Dict::new();
        let (hp, rel, person, a, b, dog, mammal, animal, rex) = (
            ex(&mut dict, "hasParent"),
            ex(&mut dict, "relatedTo"),
            ex(&mut dict, "Person"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
            ex(&mut dict, "Dog"),
            ex(&mut dict, "Mammal"),
            ex(&mut dict, "Animal"),
            ex(&mut dict, "rex"),
        );
        let (sp, dom, rng, sc, ty) = (
            iri(&mut dict, rdfs::SUB_PROPERTY_OF.as_str()),
            iri(&mut dict, rdfs::DOMAIN.as_str()),
            iri(&mut dict, rdfs::RANGE.as_str()),
            iri(&mut dict, rdfs::SUB_CLASS_OF.as_str()),
            iri(&mut dict, rdf::TYPE.as_str()),
        );
        let mut triples = vec![
            [hp, sp, rel],
            [hp, dom, person],
            [hp, rng, person],
            [a, hp, b],
            [dog, sc, mammal],
            [mammal, sc, animal],
            [rex, ty, dog],
        ];
        materialize_rdfs(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        // rdfs7/rdfs2/rdfs3 (substrate plain batch), rdfs9 (substrate type batch,
        // sq-pbz04.1.1), rdfs11 (schema-closure emission).
        assert!(set.contains(&[a, rel, b]), "rdfs7 via shared join");
        assert!(set.contains(&[a, ty, person]), "rdfs2 domain via shared join");
        assert!(set.contains(&[b, ty, person]), "rdfs3 range via shared join");
        assert!(set.contains(&[rex, ty, mammal]), "rdfs9 one hop via shared type join");
        assert!(set.contains(&[rex, ty, animal]), "rdfs9 transitive via shared type join");
        assert!(set.contains(&[dog, sc, animal]), "rdfs11 subclass transitivity");
    }

    // ---- [FABLE-5] sq-pbz04.1.1: per-branch disposition tests (rdfs9 adopt / PropExpand retain) ----

    /// The rdfs9 half of the disposition (ADOPTED): driving the shared `sparq-substrate::join`
    /// kernels for the `rdf:type`/subclass-typing join — an OBJECT-keyed probe — emits the
    /// BYTE-IDENTICAL multiset as the hand-rolled `emit_type_rdfs9` adjacency arm, over a
    /// fixture with a multi-hop closure, a class absent from the closure, and assertions
    /// sharing a class (so duplicate conclusions must be PRESERVED, not collapsed). Mirrors
    /// `substrate_join_emits_identical_plain_branch` for the rdfs2/3/7 half.
    #[cfg(feature = "substrate-join")]
    #[test]
    fn substrate_join_emits_identical_type_branch() {
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (dog, mammal, animal, plant, rex, fido, fern) = (
            ex(&mut dict, "Dog"),
            ex(&mut dict, "Mammal"),
            ex(&mut dict, "Animal"),
            ex(&mut dict, "Plant"),
            ex(&mut dict, "rex"),
            ex(&mut dict, "fido"),
            ex(&mut dict, "fern"),
        );
        // Saturated subclass closure (as `rdfs_closure` builds it): Dog ⊑* {Mammal, Animal},
        // Mammal ⊑* {Animal}. Plant appears in NO closure entry (emits nothing).
        let sc: FxHashMap<Id, Vec<Id>> =
            [(dog, vec![mammal, animal]), (mammal, vec![animal])].into_iter().collect();
        // The type-assertion partition the substrate sweep routes to the type join.
        let typed =
            vec![[rex, v.ty, dog], [fido, v.ty, dog], [rex, v.ty, mammal], [fern, v.ty, plant]];

        // Hand-rolled reference emission (the arm the default build runs).
        let mut hand = Vec::new();
        for &t in &typed {
            emit_type_rdfs9(t, &v, &sc, &mut hand);
        }
        hand.sort_unstable();

        // Shared substrate-join emission.
        let mut shared = Vec::new();
        crate::substrate_join::sweep_type_join(&typed, &sc, &mut shared);
        shared.sort_unstable();

        assert_eq!(
            shared, hand,
            "the shared substrate type join must emit the byte-identical rdfs9 multiset"
        );
        // Anchor BOTH paths to the expected rdfs9 multiset (not just to each other): note
        // `[rex, ty, animal]` appears TWICE — once via `rex a Dog`, once via `rex a Mammal` —
        // and the pre-dedup emission must preserve the duplicate exactly like the hand arm.
        let mut expect = vec![
            [rex, v.ty, mammal],
            [rex, v.ty, animal],
            [fido, v.ty, mammal],
            [fido, v.ty, animal],
            [rex, v.ty, animal],
        ];
        expect.sort_unstable();
        assert_eq!(shared, expect, "rdfs9 emission is exactly the expected multiset");
    }

    /// The PropExpand half of the disposition (RETAINED hand-rolled): pins the data-dependent
    /// per-match ORIENTATION the shared kernel cannot express — an inverse-derived predicate
    /// types the SWAPPED subject through ITS domain. `hasParent owl:inverseOf hasChild`;
    /// `hasChild rdfs:domain Child`; `a hasParent b` ⊢ `(b hasChild a)` AND `(b type Child)`
    /// — the typing lands on `b` (the swapped subject), never on `a`. Runs in BOTH feature
    /// states (under `substrate-join` the PropExpand branch is the retained per-triple path
    /// inside the substrate sweep), so a future adoption attempt that mis-handles the
    /// orientation goes red here.
    #[test]
    fn prop_expand_inverse_types_through_oriented_domain() {
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (hp, hc, child, a, b) = (
            ex(&mut dict, "hasParent"),
            ex(&mut dict, "hasChild"),
            ex(&mut dict, "Child"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let mono = MonoOwl {
            inverse: [(hp, vec![hc]), (hc, vec![hp])].into_iter().collect(),
            ..MonoOwl::default()
        };
        let mut triples = vec![[hc, v.domain, child], [a, hp, b]];
        rdfs_closure(&mut dict, &mut triples, false, &mono);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[b, hc, a]), "prp-inv: derived edge is oriented (b hasChild a)");
        assert!(
            set.contains(&[b, v.ty, child]),
            "the SWAPPED subject b is domain-typed through the derived predicate"
        );
        assert!(
            !set.contains(&[a, v.ty, child]),
            "the unswapped subject a must NOT be domain-typed — the orientation is load-bearing"
        );
    }

    /// The whole-closure pin for the disposition: a fixture driving ALL THREE sweep branches
    /// (the plain rdfs2/3/7 batch, the rdfs9 type batch, and the retained PropExpand rewrite)
    /// materialises to EXACTLY this output — full-vector equality, not membership. The SAME
    /// assertion runs in BOTH feature states (default hand-rolled vs `substrate-join`), so CI
    /// holding it green in both proves the closure is byte-identical across the join-machinery
    /// swap — the bead's invariant — with any added, dropped, or mis-oriented inference (or a
    /// double emission surviving dedup) going red.
    #[test]
    fn closure_is_byte_identical_across_join_paths() {
        let mut dict = Dict::new();
        let v = Vocab::intern(&mut dict);
        let (p, q, c, d, sym, x, y, s, o) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "q"),
            ex(&mut dict, "C"),
            ex(&mut dict, "D"),
            ex(&mut dict, "sym"),
            ex(&mut dict, "x"),
            ex(&mut dict, "y"),
            ex(&mut dict, "s"),
            ex(&mut dict, "o"),
        );
        // TBox: p ⊑ q ; q domain C ; C ⊑ D ; sym symmetric (activates PropExpand).
        // ABox: (s p o) → the PropExpand path (p is px-keyed via its super-property edge);
        //       (x sym y) → the orientation-swap path; (x type C) → the rdfs9 type path.
        let mono = MonoOwl { symmetric: [sym].into_iter().collect(), ..MonoOwl::default() };
        let asserted = vec![
            [p, v.sub_prop, q],
            [q, v.domain, c],
            [c, v.sub_class, d],
            [s, p, o],
            [x, sym, y],
            [x, v.ty, c],
        ];
        let mut triples = asserted.clone();
        rdfs_closure(&mut dict, &mut triples, false, &mono);
        // Expected output layout (documented `rdfs_closure` contract): sorted originals, then
        // the sorted derived set — (s q o) by rdfs7/prp-eqp composition, (s type C/D) by the
        // inherited-domain typing closed up the subclass graph, (y sym x) by prp-symp, and
        // (x type D) by rdfs9.
        let mut expect = asserted;
        expect.sort_unstable();
        let mut derived =
            vec![[s, q, o], [s, v.ty, c], [s, v.ty, d], [y, sym, x], [x, v.ty, d]];
        derived.sort_unstable();
        expect.extend(derived);
        assert_eq!(
            triples, expect,
            "the materialised closure must be byte-identical in both feature states"
        );
    }
}
