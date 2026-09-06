//! `why()` provers for the incremental maintenance handles (the `explain` feature; child
//! module of `incremental` so it reaches the private counting state).
//!
//! # Design: reconstruct, don't double-book
//!
//! The counting engines already keep everything that DETERMINES every derivation: the
//! asserted base, the closed TBox the counts were computed against, and the exact,
//! deterministic per-triple emission function. So a support of a derived triple — (rule,
//! premises) — is *reconstructed* on demand by running the emission trace backwards from
//! small reverse indexes over the base, rather than stored per derivation (which would be a
//! second, parallel copy of the counting bookkeeping). The explanation state this adds is
//! only: the RAW TBox edge maps (the closures flatten multi-step chains away; proofs need
//! the steps back) and subject/object reverse indexes over the base.
//!
//! # Consistency model
//!
//! `why()` reads only the CURRENT maintenance state: after any `insert`/`delete`, a
//! returned proof's leaves are asserted in the current base — a retracted support is never
//! served; if an alternative support exists it is found (the search enumerates all
//! candidate emitters), and a triple that left the closure returns `None`.
//!
//! Explanations are **a witness, not all witnesses**: one deterministic derivation per
//! triple (all choice points iterate sorted).

use super::*;
use crate::explain::{
    bfs_path, id_triple_strings, sorted, ExplainOpts, ProofBuilder, ProofTree,
};

// ════════════════════════════════════════════════════════════════════════════════════════
// RDFS — MaterializedGraph
// ════════════════════════════════════════════════════════════════════════════════════════

/// Explanation state for [`MaterializedGraph`]: raw (unclosed) TBox edges + base reverse
/// indexes. Rebuilt wholesale on TBox mutations (alongside the counts), maintained
/// incrementally on ABox deltas.
#[derive(Default)]
pub(super) struct RdfsExplain {
    /// Reverse indexes over the base. HASH-SET buckets: deletes must be O(1) per entry
    /// (Vec + position scan measured a 38x cliff on 10k-delete deltas — popular objects
    /// like a common class make buckets huge). Determinism is unaffected: `candidates()`
    /// sorts at query time.
    by_subj: FxHashMap<Id, FxHashSet<[Id; 3]>>,
    by_obj: FxHashMap<Id, FxHashSet<[Id; 3]>>,
    sc_raw: FxHashMap<Id, Vec<Id>>,
    sp_raw: FxHashMap<Id, Vec<Id>>,
    dom_raw: FxHashMap<Id, Vec<Id>>,
    rng_raw: FxHashMap<Id, Vec<Id>>,
}

impl RdfsExplain {
    pub(super) fn rebuild(
        &mut self,
        base: &FxHashSet<[Id; 3]>,
        sc: FxHashMap<Id, Vec<Id>>,
        sp: FxHashMap<Id, Vec<Id>>,
        dom: FxHashMap<Id, Vec<Id>>,
        rng: FxHashMap<Id, Vec<Id>>,
    ) {
        self.sc_raw = sc;
        self.sp_raw = sp;
        self.dom_raw = dom;
        self.rng_raw = rng;
        self.by_subj.clear();
        self.by_obj.clear();
        for &t in base {
            self.index(t);
        }
    }

    pub(super) fn add_triples(&mut self, added: &[[Id; 3]]) {
        for &t in added {
            self.index(t);
        }
    }

    pub(super) fn remove_triples(&mut self, removed: &[[Id; 3]]) {
        for t in removed {
            if let Some(v) = self.by_subj.get_mut(&t[0]) {
                v.remove(t);
            }
            if let Some(v) = self.by_obj.get_mut(&t[2]) {
                v.remove(t);
            }
        }
    }

    fn index(&mut self, t: [Id; 3]) {
        self.by_subj.entry(t[0]).or_default().insert(t);
        self.by_obj.entry(t[2]).or_default().insert(t);
    }

    /// Base triples whose emissions can have `id` as the conclusion's subject — i.e. every
    /// base triple with `id` in subject or object position (sorted + deduplicated, so the
    /// first successful trace is deterministic).
    fn candidates(&self, id: Id) -> Vec<[Id; 3]> {
        let mut c: Vec<[Id; 3]> = Vec::new();
        if let Some(v) = self.by_subj.get(&id) {
            c.extend(v.iter().copied());
        }
        if let Some(v) = self.by_obj.get(&id) {
            c.extend(v.iter().copied());
        }
        c.sort_unstable();
        c.dedup();
        c
    }
}

impl MaterializedGraph {
    /// One derivation of `t` from the current asserted base, or `None` if `t` is not in the
    /// closure (or a cap of [`ExplainOpts::default`] is exceeded). `dict` is only read (to
    /// render terms). See [`crate::explain`] for the proof shape and semantics.
    pub fn why(&self, dict: &Dict, t: [Id; 3]) -> Option<ProofTree> {
        self.why_with(dict, t, ExplainOpts::default())
    }

    /// [`why`](Self::why) with explicit depth/size caps.
    pub fn why_with(&self, dict: &Dict, t: [Id; 3], opts: ExplainOpts) -> Option<ProofTree> {
        let mut p = RdfsProver {
            g: self,
            dict,
            b: ProofBuilder::new(opts),
            memo: FxHashMap::default(),
        };
        let root = p.prove(t, 0)?;
        Some(p.b.finish(root))
    }
}

struct RdfsProver<'a> {
    g: &'a MaterializedGraph,
    dict: &'a Dict,
    b: ProofBuilder,
    memo: FxHashMap<[Id; 3], u32>,
}

impl RdfsProver<'_> {
    fn push(&mut self, t: [Id; 3], rule: &str, premises: Vec<u32>) -> Option<u32> {
        let ix = self.b.push(id_triple_strings(self.dict, t), rule, premises)?;
        self.memo.insert(t, ix);
        Some(ix)
    }

    fn prove(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        if let Some(&ix) = self.memo.get(&t) {
            return Some(ix);
        }
        if depth > self.b.opts.max_depth {
            return None;
        }
        if self.g.base.contains(&t) {
            return self.push(t, "asserted", vec![]);
        }
        if self.g.schema_facts.contains(&t) {
            return self.prove_schema(t, depth);
        }
        if self.g.counts.contains_key(&t) {
            return self.prove_emitted(t, depth);
        }
        None
    }

    /// TBox-closure facts: a `subClassOf` (rdfs11) / `subPropertyOf` (rdfs5) chain over the
    /// raw (asserted) edges, folded left-associatively.
    fn prove_schema(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        let v = &self.g.v;
        let (raw, rule) = if t[1] == v.sub_class {
            (&self.g.explain.sc_raw, "rdfs11")
        } else if t[1] == v.sub_prop {
            (&self.g.explain.sp_raw, "rdfs5")
        } else {
            return None;
        };
        let succ = |n: Id| raw.get(&n).map(|s| sorted(s)).unwrap_or_default();
        let path = if t[0] == t[2] {
            // Self-loop closure fact (c R* c via a cycle): step onto a successor first.
            sorted(&succ(t[0]))
                .into_iter()
                .find_map(|m| {
                    let mut p = vec![t[0]];
                    p.extend(bfs_path(m, t[2], succ)?);
                    Some(p)
                })?
        } else {
            bfs_path(t[0], t[2], succ)?
        };
        debug_assert!(path.len() >= 2, "a closure fact implies at least one edge");
        // First raw edge is an asserted leaf; each further edge extends the chain by one
        // rule application: (a R m), (m R n) ⊢ (a R n).
        let mut prev = self.prove([path[0], t[1], path[1]], depth + 1)?;
        for i in 2..path.len() {
            let edge = self.prove([path[i - 1], t[1], path[i]], depth + 1)?;
            let conclusion = [path[0], t[1], path[i]];
            prev = match self.memo.get(&conclusion) {
                Some(&ix) => ix,
                None => self.push(conclusion, rule, vec![prev, edge])?,
            };
        }
        Some(prev)
    }

    /// Counted (ABox-derived) facts: find an asserted emitter among the base triples
    /// touching the conclusion's subject and trace the emission back through the closed
    /// TBox, decomposing the flattened closures into single spec-rule steps.
    fn prove_emitted(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        let v = &self.g.v;
        for src in self.g.explain.candidates(t[0]) {
            let [s, p, o] = src;
            if p == v.ty {
                // rdfs9: (o subClassOf* d), (s type o) ⊢ (s type d).
                if t[1] == v.ty && t[0] == s && contains(self.g.sc_closure.get(&o), t[2]) {
                    let schema = self.prove([o, v.sub_class, t[2]], depth + 1)?;
                    let src_n = self.prove(src, depth + 1)?;
                    return self.push(t, "rdfs9", vec![schema, src_n]);
                }
                continue;
            }
            // rdfs7: (p subPropertyOf* q), (s p o) ⊢ (s q o).
            if t[0] == s && t[2] == o && contains(self.g.sp_closure.get(&p), t[1]) {
                let schema = self.prove([p, v.sub_prop, t[1]], depth + 1)?;
                let src_n = self.prove(src, depth + 1)?;
                return self.push(t, "rdfs7", vec![schema, src_n]);
            }
            if t[1] != v.ty {
                continue;
            }
            // rdfs2 (+rdfs7/rdfs9): domain typing of the subject.
            if t[0] == s && contains(self.g.dom_full.get(&p), t[2]) {
                if let Some(ix) = self.trace_typing(t, src, false, depth) {
                    return Some(ix);
                }
            }
            // rdfs3 (+rdfs7/rdfs9): range typing of the object.
            if t[0] == o && contains(self.g.rng_full.get(&p), t[2]) {
                if let Some(ix) = self.trace_typing(t, src, true, depth) {
                    return Some(ix);
                }
            }
        }
        None
    }

    /// Decompose a flattened `dom_full`/`rng_full` emission `(s p o) ⊢ (x type c)` into
    /// spec-rule steps: optional rdfs7 up to the super-property `q` carrying the raw
    /// domain/range, the rdfs2/rdfs3 application, and an optional rdfs9 up the subclass
    /// chain. `range` selects rdfs3 (typing the object) over rdfs2 (typing the subject).
    fn trace_typing(&mut self, t: [Id; 3], src: [Id; 3], range: bool, depth: usize) -> Option<u32> {
        let v = &self.g.v;
        let [s, p, o] = src;
        let (raw, dr_pred, dr_rule) = if range {
            (&self.g.explain.rng_raw, v.range, "rdfs3")
        } else {
            (&self.g.explain.dom_raw, v.domain, "rdfs2")
        };
        let c = t[2];
        // Properties whose raw domain/range can type this triple: p and its super-properties.
        let mut props = vec![p];
        if let Some(qs) = self.g.sp_closure.get(&p) {
            props.extend(qs.iter().copied());
        }
        props.sort_unstable();
        props.dedup();
        for q in props {
            let Some(c0s) = raw.get(&q) else { continue };
            for &c0 in &sorted(c0s) {
                if c0 != c && !contains(self.g.sc_closure.get(&c0), c) {
                    continue;
                }
                // (s q o): the source itself, or one rdfs7 step up.
                let edge_triple = [s, q, o];
                if edge_triple == t {
                    continue; // pathological self-reference; try the next decomposition
                }
                let edge = if q == p {
                    self.prove(src, depth + 1)?
                } else if let Some(&ix) = self.memo.get(&edge_triple) {
                    ix
                } else {
                    let schema = self.prove([p, v.sub_prop, q], depth + 1)?;
                    let src_n = self.prove(src, depth + 1)?;
                    self.push(edge_triple, "rdfs7", vec![schema, src_n])?
                };
                // rdfs2/rdfs3: (q domain/range c0), (s q o) ⊢ (x type c0).
                let dr_leaf = self.prove([q, dr_pred, c0], depth + 1)?;
                let typed_triple = [if range { o } else { s }, v.ty, c0];
                if c0 == c {
                    return self.push(t, dr_rule, vec![dr_leaf, edge]);
                }
                let typed = if let Some(&ix) = self.memo.get(&typed_triple) {
                    ix
                } else {
                    self.push(typed_triple, dr_rule, vec![dr_leaf, edge])?
                };
                // rdfs9 up the subclass closure to c.
                let schema = self.prove([c0, v.sub_class, c], depth + 1)?;
                return self.push(t, "rdfs9", vec![schema, typed]);
            }
        }
        None
    }
}

/// Is `x` in the optional closure entry?
fn contains(entry: Option<&Vec<Id>>, x: Id) -> bool {
    entry.is_some_and(|v| v.contains(&x))
}

// ════════════════════════════════════════════════════════════════════════════════════════
// OWL 2 RL — MaterializedOwlGraph
// ════════════════════════════════════════════════════════════════════════════════════════

/// Origin of a raw `subClassOf`/`subPropertyOf` edge (the engine folds equivalences and the
/// XSD numeric tower into the same TBox maps; a proof must name the real first step).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum EdgeOrigin {
    /// An asserted base triple.
    Base,
    /// Folded from this asserted `owl:equivalentClass`/`equivalentProperty` triple
    /// (scm-eqc1 / scm-eqp1).
    Equiv([Id; 3]),
    /// An XSD numeric-tower axiom (premise-free `axiom-xsd` leaf).
    Xsd,
}

/// Explanation state for [`MaterializedOwlGraph`]: raw TBox edges with origins + base
/// reverse indexes. Everything is reconstructed from ONE scan of the base (mirroring the
/// scan in `rematerialize`), so no plumbing into the maintenance internals is needed.
#[derive(Default)]
pub(super) struct OwlExplain {
    /// Reverse indexes over the base (hash-set buckets — see [`RdfsExplain`]).
    by_subj: FxHashMap<Id, FxHashSet<[Id; 3]>>,
    by_obj: FxHashMap<Id, FxHashSet<[Id; 3]>>,
    sc_raw: FxHashMap<Id, Vec<(Id, EdgeOrigin)>>,
    sp_raw: FxHashMap<Id, Vec<(Id, EdgeOrigin)>>,
    /// Base-asserted domain/range classes per property.
    dom_raw: FxHashMap<Id, Vec<Id>>,
    rng_raw: FxHashMap<Id, Vec<Id>>,
    /// Raw `owl:inverseOf` adjacency (both directions) + the asserted premise per pair.
    inv_adj: FxHashMap<Id, Vec<Id>>,
    inv_prem: FxHashMap<(Id, Id), [Id; 3]>,
    symmetric: FxHashSet<Id>,
}

impl OwlExplain {
    pub(super) fn rebuild(&mut self, base: &FxHashSet<[Id; 3]>, v: &Vocab, ow: &OwlIds) {
        *self = OwlExplain::default();
        let mut occurring: FxHashSet<Id> = FxHashSet::default();
        for &t in base {
            self.by_subj.entry(t[0]).or_default().insert(t);
            self.by_obj.entry(t[2]).or_default().insert(t);
            let [s, p, o] = t;
            if p == v.sub_class {
                self.sc_raw.entry(s).or_default().push((o, EdgeOrigin::Base));
            } else if p == v.sub_prop {
                self.sp_raw.entry(s).or_default().push((o, EdgeOrigin::Base));
            } else if p == v.domain {
                self.dom_raw.entry(s).or_default().push(o);
            } else if p == v.range {
                self.rng_raw.entry(s).or_default().push(o);
            } else if p == ow.equiv_class {
                self.sc_raw.entry(s).or_default().push((o, EdgeOrigin::Equiv(t)));
                self.sc_raw.entry(o).or_default().push((s, EdgeOrigin::Equiv(t)));
            } else if p == ow.equiv_prop {
                self.sp_raw.entry(s).or_default().push((o, EdgeOrigin::Equiv(t)));
                self.sp_raw.entry(o).or_default().push((s, EdgeOrigin::Equiv(t)));
            } else if p == ow.inverse_of {
                for (a, b) in [(s, o), (o, s)] {
                    self.inv_adj.entry(a).or_default().push(b);
                    let e = self.inv_prem.entry((a, b)).or_insert(t);
                    *e = (*e).min(t); // deterministic premise pick
                }
            } else if p == v.ty && o == ow.symmetric {
                self.symmetric.insert(s);
            }
            for id in t {
                if ow.special.contains(&id) {
                    occurring.insert(id);
                }
            }
        }
        // The occurrence-guarded XSD tower (mirrors `rematerialize` step 3).
        let mut chain: Vec<(Id, Id)> = Vec::new();
        for &(sub, sup) in &ow.xsd {
            if occurring.contains(&sub) || chain.iter().any(|&(_, b)| b == sub) {
                chain.push((sub, sup));
            }
        }
        for (sub, sup) in chain {
            self.sc_raw.entry(sub).or_default().push((sup, EdgeOrigin::Xsd));
        }
    }

    pub(super) fn add_triples(&mut self, added: &[[Id; 3]]) {
        // ABox-only by construction: TBox/axiom/special deltas rebuild instead (the
        // maintenance path enforces it), so only the reverse indexes can change here.
        for &t in added {
            self.by_subj.entry(t[0]).or_default().insert(t);
            self.by_obj.entry(t[2]).or_default().insert(t);
        }
    }

    pub(super) fn remove_triples(&mut self, removed: &[[Id; 3]]) {
        for t in removed {
            if let Some(v) = self.by_subj.get_mut(&t[0]) {
                v.remove(t);
            }
            if let Some(v) = self.by_obj.get_mut(&t[2]) {
                v.remove(t);
            }
        }
    }

    fn candidates(&self, id: Id) -> Vec<[Id; 3]> {
        let mut c: Vec<[Id; 3]> = Vec::new();
        if let Some(v) = self.by_subj.get(&id) {
            c.extend(v.iter().copied());
        }
        if let Some(v) = self.by_obj.get(&id) {
            c.extend(v.iter().copied());
        }
        c.sort_unstable();
        c.dedup();
        c
    }
}

/// How a transitive-layer fact was first derived (the traced replica of
/// `compute_virtual`). Ground effective edges (in `e0`) are absent — they are proved as
/// plain emissions of asserted triples.
#[derive(Clone, Copy)]
enum TraceStep {
    /// prp-trp: `(x r mid)`, `(mid r z)` ⊢ `(x r z)`.
    Chain(Id),
    /// A closure edge of another transitive property carried over by the
    /// property-orientation closure (prp-spo1/prp-inv/prp-symp steps applied to `src`).
    Transfer { src: [Id; 3] },
}

/// Lazily-built traces for one `why()` call (replicas of the engine's TBox fixpoints with
/// first-derivation provenance; deterministic — all iteration sorted).
#[derive(Default)]
struct OwlTraces {
    /// Virtual (transitive-layer) fact → first derivation.
    trans: Option<FxHashMap<[Id; 3], TraceStep>>,
    /// `(is_range, property, class)` → first derivation of that raw domain/range entry.
    dr: Option<FxHashMap<(bool, Id, Id), DrStep>>,
}

#[derive(Clone, Copy)]
enum DrStep {
    /// An asserted `rdfs:domain`/`rdfs:range` triple.
    Base,
    /// Transposed through `owl:inverseOf` (engine rule `inv-dom`/`inv-rng`): premise
    /// `prem` relates the properties; the class comes from `src`'s closed domain/range.
    Inv { prem: [Id; 3], src: Id, src_is_rng: bool },
}

impl MaterializedOwlGraph {
    /// One derivation of `t` from the current asserted base, or `None` if `t` is not in
    /// the closure, the graph runs in [`OwlMode::Fallback`] (recursive/equality features —
    /// no counting state to explain from; documented limitation), or a cap is exceeded.
    pub fn why(&self, dict: &Dict, t: [Id; 3]) -> Option<ProofTree> {
        self.why_with(dict, t, ExplainOpts::default())
    }

    /// [`why`](Self::why) with explicit depth/size caps.
    pub fn why_with(&self, dict: &Dict, t: [Id; 3], opts: ExplainOpts) -> Option<ProofTree> {
        if self.mode == OwlMode::Fallback && !self.base.contains(&t) {
            return None;
        }
        let mut p = OwlProver {
            g: self,
            dict,
            b: ProofBuilder::new(opts),
            memo: FxHashMap::default(),
            stack: FxHashSet::default(),
            traces: OwlTraces::default(),
        };
        let root = p.prove(t, 0)?;
        Some(p.b.finish(root))
    }
}

struct OwlProver<'a> {
    g: &'a MaterializedOwlGraph,
    dict: &'a Dict,
    b: ProofBuilder,
    memo: FxHashMap<[Id; 3], u32>,
    /// In-progress cycle guard (mutually-supporting derived facts try the next support).
    stack: FxHashSet<[Id; 3]>,
    traces: OwlTraces,
}

impl OwlProver<'_> {
    fn push(&mut self, t: [Id; 3], rule: &str, premises: Vec<u32>) -> Option<u32> {
        let ix = self.b.push(id_triple_strings(self.dict, t), rule, premises)?;
        self.memo.insert(t, ix);
        Some(ix)
    }

    fn prove(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        if let Some(&ix) = self.memo.get(&t) {
            return Some(ix);
        }
        if depth > self.b.opts.max_depth || !self.stack.insert(t) {
            return None;
        }
        let r = self.prove_inner(t, depth);
        self.stack.remove(&t);
        r
    }

    fn prove_inner(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        if self.g.base.contains(&t) {
            return self.push(t, "asserted", vec![]);
        }
        if self.g.schema_facts.contains(&t) {
            if let Some(ix) = self.prove_schema(t, depth) {
                return Some(ix);
            }
        }
        let in_virtual = self.g.virtual_facts.contains(&t);
        if self.g.counts.contains_key(&t) || in_virtual {
            if let Some(ix) = self.prove_emitted(t, depth) {
                return Some(ix);
            }
            if in_virtual {
                return self.prove_transitive(t, depth);
            }
        }
        None
    }

    // ---- TBox-derived (schema) facts ----------------------------------------------------

    fn prove_schema(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        let (v, ow) = (&self.g.v, &self.g.ow);
        if t[1] == v.sub_class {
            return self.prove_chain(t, depth, false);
        }
        if t[1] == v.sub_prop {
            return self.prove_chain(t, depth, true);
        }
        if t[1] == v.domain {
            return self.prove_dr(t[0], false, t[2], depth);
        }
        if t[1] == v.range {
            return self.prove_dr(t[0], true, t[2], depth);
        }
        // Post-equivalences: mutual subsumption ⊢ equivalence (scm-eqc2 / scm-eqp2).
        if t[1] == ow.equiv_class || t[1] == ow.equiv_prop {
            let (rel, rule) = if t[1] == ow.equiv_class {
                (v.sub_class, "scm-eqc2")
            } else {
                (v.sub_prop, "scm-eqp2")
            };
            let fwd = self.prove([t[0], rel, t[2]], depth + 1)?;
            let bwd = self.prove([t[2], rel, t[0]], depth + 1)?;
            return self.push(t, rule, vec![fwd, bwd]);
        }
        // pre_monotone owl:Thing / owl:Nothing typing (+ its cax-sco emissions).
        if t[1] == v.ty && (t[0] == ow.thing || t[0] == ow.nothing) {
            if t[2] == ow.owl_class {
                return self.push(t, "axiom-owl", vec![]);
            }
            if contains(self.g.sc_closure.get(&ow.owl_class), t[2]) {
                let schema = self.prove([ow.owl_class, v.sub_class, t[2]], depth + 1)?;
                let axiom = self.prove([t[0], v.ty, ow.owl_class], depth + 1)?;
                return self.push(t, "cax-sco", vec![schema, axiom]);
            }
        }
        None
    }

    /// `subClassOf`/`subPropertyOf` closure facts: a chain over the raw edges (asserted,
    /// equivalence-folded, or XSD-axiom), folded left-associatively with scm-sco/scm-spo.
    fn prove_chain(&mut self, t: [Id; 3], depth: usize, props: bool) -> Option<u32> {
        let raw = if props { &self.g.explain.sp_raw } else { &self.g.explain.sc_raw };
        let rule = if props { "scm-spo" } else { "scm-sco" };
        let succ = |n: Id| -> Vec<Id> {
            let mut s: Vec<Id> = raw.get(&n).map(|v| v.iter().map(|&(m, _)| m).collect()).unwrap_or_default();
            s.sort_unstable();
            s.dedup();
            s
        };
        let path = if t[0] == t[2] {
            sorted(&succ(t[0])).into_iter().find_map(|m| {
                let mut p = vec![t[0]];
                p.extend(bfs_path(m, t[2], succ)?);
                Some(p)
            })?
        } else {
            bfs_path(t[0], t[2], succ)?
        };
        debug_assert!(path.len() >= 2);
        let mut prev = self.prove_raw_edge(path[0], path[1], props, depth)?;
        for i in 2..path.len() {
            let edge = self.prove_raw_edge(path[i - 1], path[i], props, depth)?;
            let conclusion = [path[0], t[1], path[i]];
            prev = match self.memo.get(&conclusion) {
                Some(&ix) => ix,
                None => self.push(conclusion, rule, vec![prev, edge])?,
            };
        }
        Some(prev)
    }

    /// One raw `subClassOf`/`subPropertyOf` edge, by its (deterministically smallest)
    /// origin: an asserted leaf, one scm-eqc1/scm-eqp1 step, or an XSD axiom leaf.
    fn prove_raw_edge(&mut self, x: Id, y: Id, props: bool, depth: usize) -> Option<u32> {
        let (raw, pred, eq_rule) = if props {
            (&self.g.explain.sp_raw, self.g.v.sub_prop, "scm-eqp1")
        } else {
            (&self.g.explain.sc_raw, self.g.v.sub_class, "scm-eqc1")
        };
        let t = [x, pred, y];
        if let Some(&ix) = self.memo.get(&t) {
            return Some(ix);
        }
        let origin = raw
            .get(&x)?
            .iter()
            .filter(|&&(m, _)| m == y)
            .map(|&(_, o)| o)
            .min()?; // Base < Equiv < Xsd: prefer the asserted edge
        match origin {
            EdgeOrigin::Base => self.prove(t, depth + 1),
            EdgeOrigin::Equiv(e) => {
                let prem = self.prove(e, depth + 1)?;
                self.push(t, eq_rule, vec![prem])
            }
            EdgeOrigin::Xsd => self.push(t, "axiom-xsd", vec![]),
        }
    }

    /// Closed `rdfs:domain`/`rdfs:range` schema facts (`dom_used`/`rng_used`): decompose
    /// into the raw entry (asserted, or inverse-transposed — engine rules inv-dom/inv-rng)
    /// plus scm-dom2/scm-rng2 (super-property) and scm-dom1/scm-rng1 (super-class) steps.
    fn prove_dr(&mut self, p: Id, is_rng: bool, c: Id, depth: usize) -> Option<u32> {
        self.build_dr_trace();
        let v = &self.g.v;
        let dr_pred = if is_rng { v.range } else { v.domain };
        let mut props = vec![p];
        if let Some(qs) = self.g.sp_closure.get(&p) {
            props.extend(qs.iter().copied());
        }
        props.sort_unstable();
        props.dedup();
        let trace = self.traces.dr.as_ref().unwrap();
        // Collect candidate decompositions first (borrow discipline), sorted.
        let mut cands: Vec<(Id, Id, DrStep)> = Vec::new();
        for &q in &props {
            for (&(ir, tp, c0), &step) in trace.iter() {
                if ir == is_rng && tp == q && (c0 == c || contains(self.g.sc_closure.get(&c0), c)) {
                    cands.push((q, c0, step));
                }
            }
        }
        cands.sort_unstable_by_key(|&(q, c0, _)| (q, c0));
        for (q, c0, step) in cands {
            let entry_t = [q, dr_pred, c0];
            let entry = match step {
                DrStep::Base => self.prove(entry_t, depth + 1),
                DrStep::Inv { prem, src, src_is_rng } => {
                    if let Some(&ix) = self.memo.get(&entry_t) {
                        Some(ix)
                    } else {
                        let src_pred = if src_is_rng { v.range } else { v.domain };
                        let rule = if is_rng { "inv-rng" } else { "inv-dom" };
                        (|| {
                            let prem_n = self.prove(prem, depth + 1)?;
                            let src_n = self.prove([src, src_pred, c0], depth + 1)?;
                            self.push(entry_t, rule, vec![prem_n, src_n])
                        })()
                    }
                }
            };
            let Some(mut cur) = entry else { continue };
            let mut cur_t = entry_t;
            if q != p {
                let conclusion = [p, dr_pred, c0];
                let rule = if is_rng { "scm-rng2" } else { "scm-dom2" };
                cur = match self.memo.get(&conclusion) {
                    Some(&ix) => ix,
                    None => {
                        let Some(sp_n) = self.prove([p, v.sub_prop, q], depth + 1) else { continue };
                        let Some(n) = self.push(conclusion, rule, vec![cur, sp_n]) else { continue };
                        n
                    }
                };
                cur_t = conclusion;
            }
            if c0 != c {
                let conclusion = [p, dr_pred, c];
                let rule = if is_rng { "scm-rng1" } else { "scm-dom1" };
                cur = match self.memo.get(&conclusion) {
                    Some(&ix) => ix,
                    None => {
                        let Some(sc_n) = self.prove([c0, v.sub_class, c], depth + 1) else { continue };
                        let Some(n) = self.push(conclusion, rule, vec![cur, sc_n]) else { continue };
                        n
                    }
                };
                cur_t = conclusion;
            }
            debug_assert_eq!(cur_t, [p, dr_pred, c]);
            return Some(cur);
        }
        None
    }

    /// Replica of the engine's fixpoint-mode domain/range inverse-transposition loop, with
    /// first-derivation provenance (insertion order is well-founded by construction).
    fn build_dr_trace(&mut self) {
        if self.traces.dr.is_some() {
            return;
        }
        let ex = &self.g.explain;
        let mut trace: FxHashMap<(bool, Id, Id), DrStep> = FxHashMap::default();
        for (m, ir) in [(&ex.dom_raw, false), (&ex.rng_raw, true)] {
            for (&p, cs) in m {
                for &c in cs {
                    trace.insert((ir, p, c), DrStep::Base);
                }
            }
        }
        if self.g.mode == OwlMode::CountingFixpoint && !ex.inv_adj.is_empty() {
            let mut inv_pairs: Vec<(Id, Id)> = ex
                .inv_adj
                .iter()
                .flat_map(|(&p, qs)| qs.iter().map(move |&q| (p, q)))
                .collect();
            inv_pairs.sort_unstable();
            inv_pairs.dedup();
            loop {
                // Closed dom/rng of every property from the CURRENT raw trace.
                let closed = |ir: bool, trace: &FxHashMap<(bool, Id, Id), DrStep>| {
                    let mut raw: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
                    for &(r, p, c) in trace.keys() {
                        if r == ir {
                            raw.entry(p).or_default().push(c);
                        }
                    }
                    close_dr(&raw, &self.g.sp_closure, &self.g.sc_closure)
                };
                let df = closed(false, &trace);
                let rf = closed(true, &trace);
                let mut changed = false;
                for &(p, q) in &inv_pairs {
                    let prem = ex.inv_prem[&(p, q)];
                    if let Some(cs) = df.get(&p) {
                        for &c in &sorted(cs) {
                            trace.entry((true, q, c)).or_insert_with(|| {
                                changed = true;
                                DrStep::Inv { prem, src: p, src_is_rng: false }
                            });
                        }
                    }
                    if let Some(cs) = rf.get(&p) {
                        for &c in &sorted(cs) {
                            trace.entry((false, q, c)).or_insert_with(|| {
                                changed = true;
                                DrStep::Inv { prem, src: p, src_is_rng: true }
                            });
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }
        self.traces.dr = Some(trace);
    }

    // ---- ABox emissions -------------------------------------------------------------------

    /// Counted facts: find an emitter among the asserted triples (and transitive-layer
    /// facts) touching the conclusion's subject, and trace the emission back through the
    /// closed TBox as single spec-rule steps.
    fn prove_emitted(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        let mut cands = self.g.explain.candidates(t[0]);
        let mut virt: Vec<[Id; 3]> = self
            .g
            .virtual_facts
            .iter()
            .filter(|f| (f[0] == t[0] || f[2] == t[0]) && **f != t)
            .copied()
            .collect();
        virt.sort_unstable();
        cands.extend(virt);
        for src in cands {
            if src == t || self.stack.contains(&src) {
                continue;
            }
            if let Some(ix) = self.trace_emit(src, t, depth) {
                return Some(ix);
            }
        }
        None
    }

    /// Does `src`'s emission set contain `t`? If so, build the proof for that emission.
    fn trace_emit(&mut self, src: [Id; 3], t: [Id; 3], depth: usize) -> Option<u32> {
        let (v, ow) = (&self.g.v, &self.g.ow);
        let [s, p, o] = src;
        if p == v.ty {
            // cax-sco: (o subClassOf* d), (s type o) ⊢ (s type d).
            if t[1] == v.ty && t[0] == s && contains(self.g.sc_closure.get(&o), t[2]) {
                let schema = self.prove([o, v.sub_class, t[2]], depth + 1)?;
                let src_n = self.prove(src, depth + 1)?;
                return self.push(t, "cax-sco", vec![schema, src_n]);
            }
            return None;
        }
        // Engine rule sym-dif: (x differentFrom y) ⊢ (y differentFrom x).
        if p == ow.different_from && t == [o, p, s] {
            let src_n = self.prove(src, depth + 1)?;
            return self.push(t, "sym-dif", vec![src_n]);
        }
        if self.g.px_active {
            // Property-orientation emissions (mirrors emit_std's px path incl. the
            // identity fall-through for px-absent properties).
            let identity = [(p, false)];
            let entries: Vec<(Id, bool)> = match self.g.px.get(&p) {
                Some(e) if !e.is_empty() => sorted(e),
                _ => identity.to_vec(),
            };
            for (r, swap) in entries {
                let (rs, ro) = if swap { (o, s) } else { (s, o) };
                if t == [rs, r, ro] && (r != p || swap) {
                    let src_n = self.prove(src, depth + 1)?;
                    return self.prove_oriented(src_n, src, (r, swap), depth);
                }
                if t[1] == v.ty {
                    if t[0] == rs && contains(self.g.dom_used.get(&r), t[2]) {
                        if let Some(ix) = self.trace_typing_owl(t, src, (r, swap), false, depth) {
                            return Some(ix);
                        }
                    }
                    if t[0] == ro && contains(self.g.rng_used.get(&r), t[2]) {
                        if let Some(ix) = self.trace_typing_owl(t, src, (r, swap), true, depth) {
                            return Some(ix);
                        }
                    }
                }
            }
            return None;
        }
        // Plain (no-orientation) path: subPropertyOf rewrite + domain/range typing.
        if t[0] == s && t[2] == o && contains(self.g.sp_closure.get(&p), t[1]) {
            let schema = self.prove([p, v.sub_prop, t[1]], depth + 1)?;
            let src_n = self.prove(src, depth + 1)?;
            return self.push(t, "prp-spo1", vec![schema, src_n]);
        }
        if t[1] == v.ty {
            if t[0] == s && contains(self.g.dom_used.get(&p), t[2]) {
                let dr = self.prove([p, v.domain, t[2]], depth + 1)?;
                let src_n = self.prove(src, depth + 1)?;
                return self.push(t, "prp-dom", vec![dr, src_n]);
            }
            if t[0] == o && contains(self.g.rng_used.get(&p), t[2]) {
                let dr = self.prove([p, v.range, t[2]], depth + 1)?;
                let src_n = self.prove(src, depth + 1)?;
                return self.push(t, "prp-rng", vec![dr, src_n]);
            }
        }
        None
    }

    /// prp-dom / prp-rng on the (possibly re-oriented) edge `src → (r, swap)`.
    fn trace_typing_owl(
        &mut self,
        t: [Id; 3],
        src: [Id; 3],
        to: (Id, bool),
        range: bool,
        depth: usize,
    ) -> Option<u32> {
        let v = &self.g.v;
        let [s, p, o] = src;
        let (r, swap) = to;
        let (rs, ro) = if swap { (o, s) } else { (s, o) };
        let edge_t = [rs, r, ro];
        let edge = if (r, swap) == (p, false) {
            self.prove(src, depth + 1)?
        } else if let Some(&ix) = self.memo.get(&edge_t) {
            ix
        } else {
            let src_n = self.prove(src, depth + 1)?;
            self.prove_oriented(src_n, src, to, depth)?
        };
        let (dr_pred, rule) = if range { (v.range, "prp-rng") } else { (v.domain, "prp-dom") };
        let dr = self.prove([r, dr_pred, t[2]], depth + 1)?;
        self.push(t, rule, vec![dr, edge])
    }

    /// Re-orient an edge along the raw property-orientation graph from `(p, false)` to
    /// `to`, one rule application per step (prp-spo1 / prp-eqp1/2 / prp-inv1/2 /
    /// prp-symp). `src_n` proves `src`; returns the node concluding the re-oriented edge.
    fn prove_oriented(&mut self, src_n: u32, src: [Id; 3], to: (Id, bool), depth: usize) -> Option<u32> {
        let ex = &self.g.explain;
        let succ = |(q, or): (Id, bool)| -> Vec<(Id, bool)> {
            let mut out: Vec<(Id, bool)> = Vec::new();
            if let Some(es) = ex.sp_raw.get(&q) {
                out.extend(es.iter().map(|&(m, _)| (m, or)));
            }
            if let Some(es) = ex.inv_adj.get(&q) {
                out.extend(es.iter().map(|&m| (m, !or)));
            }
            if ex.symmetric.contains(&q) {
                out.push((q, !or));
            }
            out.sort_unstable();
            out.dedup();
            out
        };
        let path = bfs_path((src[1], false), to, succ)?;
        let mut cur_n = src_n;
        let mut cur = src;
        for w in path.windows(2) {
            let ((qa, oa), (qb, ob)) = (w[0], w[1]);
            let new_t = if oa == ob { [cur[0], qb, cur[2]] } else { [cur[2], qb, cur[0]] };
            if let Some(&ix) = self.memo.get(&new_t) {
                cur_n = ix;
                cur = new_t;
                continue;
            }
            // Determine the step's rule + TBox premise (deterministic priority).
            let (rule, prem): (&str, [Id; 3]) = if oa == ob {
                let origin = self
                    .g
                    .explain
                    .sp_raw
                    .get(&qa)?
                    .iter()
                    .filter(|&&(m, _)| m == qb)
                    .map(|&(_, o)| o)
                    .min()?;
                match origin {
                    EdgeOrigin::Base => ("prp-spo1", [qa, self.g.v.sub_prop, qb]),
                    EdgeOrigin::Equiv(e) => {
                        (if e[0] == qa { "prp-eqp1" } else { "prp-eqp2" }, e)
                    }
                    EdgeOrigin::Xsd => return None, // XSD edges are classes, never properties
                }
            } else if let Some(&prem) = self.g.explain.inv_prem.get(&(qa, qb)) {
                (if prem[0] == qa && prem[2] == qb { "prp-inv1" } else { "prp-inv2" }, prem)
            } else if qa == qb && self.g.explain.symmetric.contains(&qa) {
                ("prp-symp", [qa, self.g.v.ty, self.g.ow.symmetric])
            } else {
                return None;
            };
            let prem_n = self.prove(prem, depth + 1)?; // TBox premise: an asserted leaf
            cur_n = self.push(new_t, rule, vec![prem_n, cur_n])?;
            cur = new_t;
        }
        Some(cur_n)
    }

    // ---- Transitive layer -------------------------------------------------------------

    /// prp-trp facts: prove via the traced replica of `compute_virtual` (first-derivation
    /// provenance; insertion order is well-founded by construction).
    fn prove_transitive(&mut self, t: [Id; 3], depth: usize) -> Option<u32> {
        self.build_trans_trace();
        let step = *self.traces.trans.as_ref().unwrap().get(&t)?;
        match step {
            TraceStep::Chain(mid) => {
                let r = t[1];
                let typing = self.prove([r, self.g.v.ty, self.g.ow.transitive], depth + 1)?;
                let left = self.prove([t[0], r, mid], depth + 1)?;
                let right = self.prove([mid, r, t[2]], depth + 1)?;
                self.push(t, "prp-trp", vec![typing, left, right])
            }
            TraceStep::Transfer { src } => {
                // src is a closure edge of another transitive property; the orientation
                // (which (r2, swap) carried it here) is recomputed from the conclusion.
                let swap = if [src[0], t[1], src[2]] == t {
                    false
                } else if [src[2], t[1], src[0]] == t {
                    true
                } else {
                    return None;
                };
                let src_n = self.prove(src, depth + 1)?;
                self.prove_oriented(src_n, src, (t[1], swap), depth)
            }
        }
    }

    /// Deterministic replica of `compute_virtual` recording each fact's FIRST derivation.
    /// Ground `e0` edges get no entry (they are emissions of asserted triples); recorded
    /// premises always precede their conclusions, so proofs from the trace terminate.
    fn build_trans_trace(&mut self) {
        if self.traces.trans.is_some() {
            return;
        }
        let g = self.g;
        let mut trace: FxHashMap<[Id; 3], TraceStep> = FxHashMap::default();
        let mut order: Vec<Id> = g.transitive.iter().copied().collect();
        order.sort_unstable();
        let mut edges: FxHashMap<Id, FxHashSet<(Id, Id)>> = FxHashMap::default();
        for &r in &order {
            edges.insert(
                r,
                g.e0.get(&r).map(|m| m.keys().copied().collect()).unwrap_or_default(),
            );
        }
        loop {
            let mut changed = false;
            for &r in &order {
                // Traced transitive closure of edges[r] (sorted BFS per source).
                let e = &edges[&r];
                let mut adj: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
                for &(x, y) in e {
                    adj.entry(x).or_default().push(y);
                }
                for v in adj.values_mut() {
                    v.sort_unstable();
                }
                let mut closed: FxHashSet<(Id, Id)> = e.clone();
                let mut srcs: Vec<Id> = adj.keys().copied().collect();
                srcs.sort_unstable();
                for &src in &srcs {
                    // BFS from src; parent edges live in `closed`-so-far or `e`.
                    let mut seen: FxHashSet<Id> = FxHashSet::default();
                    let mut queue: std::collections::VecDeque<Id> = adj[&src].iter().copied().collect();
                    for &m in &adj[&src] {
                        seen.insert(m);
                    }
                    while let Some(m) = queue.pop_front() {
                        for &n in adj.get(&m).map(|v| v.as_slice()).unwrap_or(&[]) {
                            if seen.insert(n) {
                                if closed.insert((src, n)) {
                                    trace.entry([src, r, n]).or_insert(TraceStep::Chain(m));
                                }
                                queue.push_back(n);
                            } else if !closed.contains(&(src, n)) && closed.insert((src, n)) {
                                trace.entry([src, r, n]).or_insert(TraceStep::Chain(m));
                            }
                        }
                    }
                }
                if closed.len() != edges[&r].len() {
                    changed = true;
                }
                // Transfers to other transitive properties (sorted, deterministic).
                let mut transfers: Vec<(Id, (Id, Id), [Id; 3])> = Vec::new();
                if let Some(tr) = g.trans_transfer.get(&r) {
                    let mut tr = tr.clone();
                    tr.sort_unstable();
                    let mut closed_sorted: Vec<(Id, Id)> = closed.iter().copied().collect();
                    closed_sorted.sort_unstable();
                    for &(r2, swap) in &tr {
                        for &(x, z) in &closed_sorted {
                            let e2 = if swap { (z, x) } else { (x, z) };
                            transfers.push((r2, e2, [x, r, z]));
                        }
                    }
                }
                edges.insert(r, closed);
                for (r2, e2, src_t) in transfers {
                    if edges.entry(r2).or_default().insert(e2) {
                        trace
                            .entry([e2.0, r2, e2.1])
                            .or_insert(TraceStep::Transfer { src: src_t });
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        debug_assert_eq!(
            {
                let mut out: FxHashSet<[Id; 3]> = FxHashSet::default();
                for (r, es) in &edges {
                    out.extend(es.iter().map(|&(x, z)| [x, *r, z]));
                }
                out
            },
            g.virtual_facts,
            "traced transitive layer diverged from the engine's"
        );
        self.traces.trans = Some(trace);
    }
}

// ════════════════════════════════════════════════════════════════════════════════════════
// N3 — MaterializedN3Graph
// ════════════════════════════════════════════════════════════════════════════════════════

/// Render an N3 term as its serialized form (the engine's own writer).
fn n3_term_string(t: &N3Term) -> String {
    let mut s = String::new();
    crate::n3::serialize::write_term(t, &mut s);
    s
}

impl MaterializedN3Graph {
    /// One derivation of `fact` from the current asserted base under the graph's rules, or
    /// `None` if `fact` is not in the closure (or cannot be matched to a derivation — e.g.
    /// conclusions containing freshly-minted existential blanks, whose labels differ
    /// between runs; documented limitation).
    ///
    /// Unlike the RDFS/OWL siblings there is no incremental bookkeeping to reconstruct
    /// from (counted N3 firings are rule-and-binding specific), so this RE-RUNS the batch
    /// engine with proof recording over the rules + the current base — cost is a full
    /// materialization per call, paid only when asked. Rules are identified as
    /// `n3-rule-<i>`, `i` indexing the rules document's forward rules in order.
    pub fn why(&self, fact: &[N3Term; 3]) -> Option<ProofTree> {
        self.why_with(fact, ExplainOpts::default())
    }

    /// [`why`](Self::why) with explicit depth/size caps.
    pub fn why_with(&self, fact: &[N3Term; 3], opts: ExplainOpts) -> Option<ProofTree> {
        if !self.contains(fact) {
            return None;
        }
        let mut b = ProofBuilder::new(opts);
        if self.base.contains(fact) {
            let root = b.push(fact.clone().map(|t| n3_term_string(&t)), "asserted", vec![])?;
            return Some(b.finish(root));
        }
        // Deterministic re-derivation: serialize the base SORTED so rule-firing order (and
        // therefore the chosen witness) is stable across calls.
        let mut lines: Vec<String> = self.base.iter().map(|f| n3_serialize(std::iter::once(f))).collect();
        lines.sort_unstable();
        let src = format!("{}\n{}", self.rules_src, lines.concat());
        let (_facts, steps) = crate::n3::reason_n3_terms_proof(&src).ok()?;
        // One step per derived fact (first derivation wins).
        let mut step_map: FxHashMap<&[N3Term; 3], (usize, &[[N3Term; 3]])> = FxHashMap::default();
        for (conclusion, rule, premises) in &steps {
            step_map.entry(conclusion).or_insert((*rule, premises.as_slice()));
        }
        let mut p = N3Prover {
            base: &self.base,
            step_map,
            b,
            memo: FxHashMap::default(),
            stack: FxHashSet::default(),
        };
        let root = p.prove(fact, 0)?;
        Some(p.b.finish(root))
    }
}

struct N3Prover<'a> {
    base: &'a FxHashSet<[N3Term; 3]>,
    step_map: FxHashMap<&'a [N3Term; 3], (usize, &'a [[N3Term; 3]])>,
    b: ProofBuilder,
    memo: FxHashMap<[N3Term; 3], u32>,
    stack: FxHashSet<[N3Term; 3]>,
}

impl N3Prover<'_> {
    fn prove(&mut self, f: &[N3Term; 3], depth: usize) -> Option<u32> {
        if let Some(&ix) = self.memo.get(f) {
            return Some(ix);
        }
        if depth > self.b.opts.max_depth || !self.stack.insert(f.clone()) {
            return None;
        }
        let r = self.prove_inner(f, depth);
        self.stack.remove(f);
        r
    }

    fn prove_inner(&mut self, f: &[N3Term; 3], depth: usize) -> Option<u32> {
        let rendered = f.clone().map(|t| n3_term_string(&t));
        if self.base.contains(f) {
            let ix = self.b.push(rendered, "asserted", vec![])?;
            self.memo.insert(f.clone(), ix);
            return Some(ix);
        }
        let (rule, premises) = *self.step_map.get(f)?;
        let mut prem_nodes = Vec::with_capacity(premises.len());
        for p in premises {
            prem_nodes.push(self.prove(p, depth + 1)?);
        }
        let ix = self.b.push(rendered, &format!("n3-rule-{rule}"), prem_nodes)?;
        self.memo.insert(f.clone(), ix);
        Some(ix)
    }
}
