//! Semi-naive RDFS fixpoint for META schemas (#5090): a rule can derive a schema triple, or
//! `rdf:type` carries schema (see [`super::schema_is_meta`]), so the schema-saturate-then-sweep
//! single pass is not complete. Used by the batch closure ([`super::closure_derived`], when no
//! monotone-OWL axioms are in play) and by the incremental `MaterializedGraph`.
//!
//! Every fact is processed exactly once against indexes of the facts seen so far, so an
//! insert costs work proportional to the consequences it adds, never a recomputation.
//!
//! The rules that walk a hierarchy — rdfs5 / rdfs11 (transitivity), rdfs7 (sub-property
//! rewrite) and rdfs9 (subclass typing) — are evaluated LINEARLY: they step along raw schema
//! EDGES only, never along derived closure facts. An edge is an asserted `subClassOf` /
//! `subPropertyOf` triple or one a non-transitivity rule (rdfs7) derived. A closure fact
//! `(a R b)` always implies an edge path from `a` to `b`, so stepping along edges reaches the
//! same fixpoint, while a subClassOf chain of `n` classes costs `O(n * edges)` joins instead
//! of the `O(n^3)` a closure-by-closure join (or re-saturating a dense closure) costs; typing
//! an instance likewise walks its class's edges, not every superclass of every class.
//!
//! The evaluation is a datalog program over two relations, facts and edges (an edge is also
//! a fact). Edges are tracked exactly: a fact first derived by transitivity and later
//! asserted or derived by rdfs7 becomes an edge then. So the state is a function of the
//! asserted triples alone, which [`MetaClosure::delete`] relies on.
//!
//! **Deletes** (#6784) use DRed (delete and rederive), so their cost follows the affected
//! consequences, not the closure size:
//! 1. *Overdelete*: every fact that some rule instance derives from a deleted fact, followed
//!    transitively, using the same joins as an insert (against the old indexes).
//! 2. *Rederive*: an overdeleted fact comes back if it is still asserted, or if a rule
//!    instance derives it from the remaining facts (one backward check per fact).
//! 3. *Propagate*: the facts that came back are processed like an insert.
//!
//! With the `explain` feature each derived fact keeps its FIRST derivation (rule + the two
//! premises). Premises were facts before it, so the provenance is acyclic and replayable. A
//! fact that survives a delete keeps a derivation whose premises survive too: overdeletion
//! follows every rule instance, the recorded one included. A rederived fact records the
//! derivation that brought it back.

use crate::Vocab;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;

type T = [Id; 3];
type Derivation = (&'static str, [T; 2]);
type Index = FxHashMap<Id, Vec<Id>>;

/// One unit of semi-naive work: process `t` as a fact (index it, join it in the fact role)
/// and/or as an edge (index it, join it in the edge role).
type Work = (T, bool, bool);

#[derive(Default)]
pub(crate) struct MetaClosure {
    /// Every fact in the closure (asserted and derived).
    pub(crate) facts: FxHashSet<T>,
    /// The `subClassOf` / `subPropertyOf` facts that are edges (asserted, or derived by
    /// rdfs7), including ones queued but not processed yet.
    edges: FxHashSet<T>,
    /// `x -> [a]` for every subClassOf / subPropertyOf fact `(a R x)` (closure or edge).
    sc_in: Index,
    sp_in: Index,
    /// `a -> [b]` and `b -> [a]` for every edge `(a R b)`.
    sc_edges_out: Index,
    sp_edges_out: Index,
    sc_edges_in: Index,
    sp_edges_in: Index,
    /// `p -> [c]` for every `(p rdfs:domain c)` / `(p rdfs:range c)`, and the inverses.
    dom: Index,
    rng: Index,
    dom_inv: Index,
    rng_inv: Index,
    /// `c -> [s]` for every `(s rdf:type c)`.
    type_sub: Index,
    /// `p -> s -> [o]` for every fact.
    po: FxHashMap<Id, Index>,
    /// `p -> o -> [s]`, kept for every predicate that has (or once had) a range. A
    /// rederivation of an rdfs3 typing looks it up.
    op: FxHashMap<Id, Index>,
    /// First derivation of every derived fact.
    #[cfg(feature = "explain")]
    pub(crate) prov: FxHashMap<T, Derivation>,
}

impl MetaClosure {
    pub(crate) fn build(base: &FxHashSet<T>, v: &Vocab) -> MetaClosure {
        let mut m = MetaClosure::default();
        let base: Vec<T> = base.iter().copied().collect();
        m.insert(&base, v);
        m
    }

    /// Add asserted triples and their consequences.
    pub(crate) fn insert(&mut self, triples: &[T], v: &Vocab) {
        let mut work: Vec<Work> = Vec::new();
        for &t in triples {
            self.offer(t, true, None, &mut work, v);
        }
        self.propagate(work, v);
    }

    /// Retract asserted triples by DRed (see the module docs). `base` is the asserted set
    /// AFTER the delete.
    pub(crate) fn delete(&mut self, removed: &[T], base: &FxHashSet<T>, v: &Vocab) {
        // 1. Overdelete: follow every rule instance forward from the deleted facts, joining
        //    against the indexes as they stand (the old closure).
        let mut gone: Vec<T> = Vec::new();
        let mut seen: FxHashSet<T> = FxHashSet::default();
        for t in removed {
            if self.facts.contains(t) && seen.insert(*t) {
                gone.push(*t);
            }
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < gone.len() {
            let t = gone[i];
            i += 1;
            let edge = self.edges.contains(&t);
            self.join(t, true, edge, &mut out, v);
            for (c, _, _) in out.drain(..) {
                if self.facts.contains(&c) && seen.insert(c) {
                    gone.push(c);
                }
            }
        }
        // 2. Remove the overdeleted facts from the closure and every index.
        self.unindex(&gone, v);
        // 3. Rederive: still asserted, or one rule step from the remaining facts.
        let mut work: Vec<Work> = Vec::new();
        for &t in &gone {
            if base.contains(&t) {
                self.offer(t, true, None, &mut work, v);
            } else if let Some(why) = self.derivation(t, v) {
                let edge = why.0 == "rdfs7";
                self.offer(t, edge, Some(why), &mut work, v);
            }
        }
        // 4. Propagate the rederived facts like an insert.
        self.propagate(work, v);
    }

    fn propagate(&mut self, mut work: Vec<Work>, v: &Vocab) {
        let mut out = Vec::new();
        while let Some((t, fact, edge)) = work.pop() {
            self.index(t, fact, edge, v);
            self.join(t, fact, edge, &mut out, v);
            for (c, e, why) in out.drain(..) {
                self.offer(c, e, Some(why), &mut work, v);
            }
        }
    }

    /// A derivation of `t` enters the closure; `edge` (only meaningful for subClassOf /
    /// subPropertyOf) marks an edge. A known fact that only now becomes an edge is queued
    /// for its edge role alone.
    #[inline]
    fn offer(
        &mut self,
        t: T,
        edge: bool,
        _why: Option<Derivation>,
        work: &mut Vec<Work>,
        v: &Vocab,
    ) {
        let edge = edge && (t[1] == v.sub_class || t[1] == v.sub_prop);
        if self.facts.insert(t) {
            #[cfg(feature = "explain")]
            if let Some(w) = _why {
                self.prov.insert(t, w);
            }
            if edge {
                self.edges.insert(t);
            }
            work.push((t, true, edge));
        } else if edge && self.edges.insert(t) {
            work.push((t, false, true));
        }
    }

    /// Index `t` in its fact role and/or its edge role.
    fn index(&mut self, t: T, fact: bool, edge: bool, v: &Vocab) {
        let [s, p, o] = t;
        if fact {
            self.po.entry(p).or_default().entry(s).or_default().push(o);
            if let Some(m) = self.op.get_mut(&p) {
                m.entry(o).or_default().push(s);
            }
            if p == v.sub_prop {
                self.sp_in.entry(o).or_default().push(s);
            } else if p == v.sub_class {
                self.sc_in.entry(o).or_default().push(s);
            } else if p == v.domain {
                self.dom.entry(s).or_default().push(o);
                self.dom_inv.entry(o).or_default().push(s);
            } else if p == v.range {
                self.rng.entry(s).or_default().push(o);
                self.rng_inv.entry(o).or_default().push(s);
                if !self.op.contains_key(&s) {
                    let mut m = Index::default();
                    for (&x, ys) in self.po.get(&s).into_iter().flatten() {
                        for &y in ys {
                            m.entry(y).or_default().push(x);
                        }
                    }
                    self.op.insert(s, m);
                }
            } else if p == v.ty {
                self.type_sub.entry(o).or_default().push(s);
            }
        }
        if edge {
            if p == v.sub_prop {
                self.sp_edges_out.entry(s).or_default().push(o);
                self.sp_edges_in.entry(o).or_default().push(s);
            } else {
                self.sc_edges_out.entry(s).or_default().push(o);
                self.sc_edges_in.entry(o).or_default().push(s);
            }
        }
    }

    /// Every rule instance with `t` as a premise (in its fact role and/or its edge role)
    /// whose other premise is indexed: `(conclusion, conclusion is an edge, derivation)`.
    fn join(&self, t: T, fact: bool, edge: bool, out: &mut Vec<(T, bool, Derivation)>, v: &Vocab) {
        let [s, p, o] = t;
        if fact {
            // `t` as the data premise (any predicate, the schema ones included).
            if let Some(qs) = self.sp_edges_out.get(&p) {
                out.extend(
                    qs.iter()
                        .map(|&q| ([s, q, o], true, ("rdfs7", [[p, v.sub_prop, q], t]))),
                );
            }
            if let Some(cs) = self.dom.get(&p) {
                out.extend(
                    cs.iter()
                        .map(|&c| ([s, v.ty, c], true, ("rdfs2", [[p, v.domain, c], t]))),
                );
            }
            if let Some(cs) = self.rng.get(&p) {
                out.extend(
                    cs.iter()
                        .map(|&c| ([o, v.ty, c], true, ("rdfs3", [[p, v.range, c], t]))),
                );
            }
            if p == v.ty {
                if let Some(ds) = self.sc_edges_out.get(&o) {
                    out.extend(
                        ds.iter()
                            .map(|&d| ([s, v.ty, d], true, ("rdfs9", [[o, v.sub_class, d], t]))),
                    );
                }
            }
            // `t` as a schema premise.
            if p == v.sub_prop || p == v.sub_class {
                let (edges_out, rule) = if p == v.sub_class {
                    (&self.sc_edges_out, "rdfs11")
                } else {
                    (&self.sp_edges_out, "rdfs5")
                };
                // Closure side: (s R o), edge (o R x) ⊢ (s R x).
                if let Some(xs) = edges_out.get(&o) {
                    out.extend(
                        xs.iter()
                            .map(|&x| ([s, p, x], false, (rule, [t, [o, p, x]]))),
                    );
                }
            } else if p == v.domain || p == v.range {
                let (rule, range) = if p == v.domain {
                    ("rdfs2", false)
                } else {
                    ("rdfs3", true)
                };
                for (&x, ys) in self.po.get(&s).into_iter().flatten() {
                    out.extend(ys.iter().map(|&y| {
                        (
                            [if range { y } else { x }, v.ty, o],
                            true,
                            (rule, [t, [x, s, y]]),
                        )
                    }));
                }
            }
        }
        if edge {
            let (ins, rule) = if p == v.sub_class {
                (&self.sc_in, "rdfs11")
            } else {
                (&self.sp_in, "rdfs5")
            };
            // Edge side: fact (x R s), edge (s R o) ⊢ (x R o).
            if let Some(xs) = ins.get(&s) {
                out.extend(
                    xs.iter()
                        .map(|&x| ([x, p, o], false, (rule, [[x, p, s], t]))),
                );
            }
            if p == v.sub_prop {
                // rdfs7: (x s y), edge (s sp o) ⊢ (x o y).
                for (&x, ys) in self.po.get(&s).into_iter().flatten() {
                    out.extend(
                        ys.iter()
                            .map(|&y| ([x, o, y], true, ("rdfs7", [t, [x, s, y]]))),
                    );
                }
            } else if let Some(ys) = self.type_sub.get(&s) {
                // rdfs9: (y type s), edge (s sc o) ⊢ (y type o).
                out.extend(
                    ys.iter()
                        .map(|&y| ([y, v.ty, o], true, ("rdfs9", [t, [y, v.ty, s]]))),
                );
            }
        }
    }

    /// One rule instance that derives `t` from the indexed facts, or `None`. rdfs7 is tried
    /// first, because it makes `t` an edge.
    fn derivation(&self, t: T, v: &Vocab) -> Option<Derivation> {
        let [s, p, o] = t;
        let has = |x: T| self.facts.contains(&x);
        // rdfs7: edge (q sp p), fact (s q o).
        for &q in self.sp_edges_in.get(&p).into_iter().flatten() {
            if has([s, q, o]) {
                return Some(("rdfs7", [[q, v.sub_prop, p], [s, q, o]]));
            }
        }
        if p == v.ty {
            // rdfs9: edge (c sc o), fact (s type c).
            for &c in self.sc_edges_in.get(&o).into_iter().flatten() {
                if has([s, v.ty, c]) {
                    return Some(("rdfs9", [[c, v.sub_class, o], [s, v.ty, c]]));
                }
            }
            // rdfs2: (r domain o), some fact (s r y).
            for &r in self.dom_inv.get(&o).into_iter().flatten() {
                let y = self.po.get(&r).and_then(|m| m.get(&s)).and_then(|ys| ys.first());
                if let Some(&y) = y {
                    return Some(("rdfs2", [[r, v.domain, o], [s, r, y]]));
                }
            }
            // rdfs3: (r range o), some fact (x r s).
            for &r in self.rng_inv.get(&o).into_iter().flatten() {
                let x = self.op.get(&r).and_then(|m| m.get(&s)).and_then(|xs| xs.first());
                if let Some(&x) = x {
                    return Some(("rdfs3", [[r, v.range, o], [x, r, s]]));
                }
            }
        } else if p == v.sub_class || p == v.sub_prop {
            // Transitivity: fact (s R m), edge (m R o).
            let (edges_in, rule) = if p == v.sub_class {
                (&self.sc_edges_in, "rdfs11")
            } else {
                (&self.sp_edges_in, "rdfs5")
            };
            for &m in edges_in.get(&o).into_iter().flatten() {
                if has([s, p, m]) {
                    return Some((rule, [[s, p, m], [m, p, o]]));
                }
            }
        }
        None
    }

    /// Drop `gone` from the closure and from every index. Removals are grouped by index key,
    /// so each touched bucket is filtered once.
    fn unindex(&mut self, gone: &[T], v: &Vocab) {
        const SC_IN: usize = 0;
        const SP_IN: usize = 1;
        const SC_OUT: usize = 2;
        const SP_OUT: usize = 3;
        const SC_EIN: usize = 4;
        const SP_EIN: usize = 5;
        const DOM: usize = 6;
        const RNG: usize = 7;
        const DOM_INV: usize = 8;
        const RNG_INV: usize = 9;
        const TYPE_SUB: usize = 10;
        let mut buckets: [Vec<(Id, Id)>; 11] = Default::default();
        let mut po: Vec<(Id, Id, Id)> = Vec::with_capacity(gone.len());
        let mut op: Vec<(Id, Id, Id)> = Vec::new();
        for &t in gone {
            let [s, p, o] = t;
            self.facts.remove(&t);
            #[cfg(feature = "explain")]
            self.prov.remove(&t);
            po.push((p, s, o));
            if self.op.contains_key(&p) {
                op.push((p, o, s));
            }
            if p == v.sub_prop {
                buckets[SP_IN].push((o, s));
            } else if p == v.sub_class {
                buckets[SC_IN].push((o, s));
            } else if p == v.domain {
                buckets[DOM].push((s, o));
                buckets[DOM_INV].push((o, s));
            } else if p == v.range {
                buckets[RNG].push((s, o));
                buckets[RNG_INV].push((o, s));
            } else if p == v.ty {
                buckets[TYPE_SUB].push((o, s));
            }
            if self.edges.remove(&t) {
                if p == v.sub_prop {
                    buckets[SP_OUT].push((s, o));
                    buckets[SP_EIN].push((o, s));
                } else {
                    buckets[SC_OUT].push((s, o));
                    buckets[SC_EIN].push((o, s));
                }
            }
        }
        let indexes: [&mut Index; 11] = [
            &mut self.sc_in,
            &mut self.sp_in,
            &mut self.sc_edges_out,
            &mut self.sp_edges_out,
            &mut self.sc_edges_in,
            &mut self.sp_edges_in,
            &mut self.dom,
            &mut self.rng,
            &mut self.dom_inv,
            &mut self.rng_inv,
            &mut self.type_sub,
        ];
        for (index, mut pairs) in indexes.into_iter().zip(buckets) {
            remove_pairs(index, &mut pairs);
        }
        remove_nested(&mut self.po, po, true);
        // An `op` entry stays even when emptied: its predicate keeps an object index.
        remove_nested(&mut self.op, op, false);
    }
}

/// Remove each `(key, value)` of `pairs` from `index`. Each touched bucket is filtered once,
/// and an emptied bucket is dropped.
fn remove_pairs(index: &mut Index, pairs: &mut [(Id, Id)]) {
    pairs.sort_unstable();
    for group in pairs.chunk_by(|a, b| a.0 == b.0) {
        let key = group[0].0;
        let Some(bucket) = index.get_mut(&key) else {
            continue;
        };
        if let [(_, x)] = group {
            if let Some(i) = bucket.iter().position(|y| y == x) {
                bucket.swap_remove(i);
            }
        } else {
            bucket.retain(|y| group.binary_search(&(key, *y)).is_err());
        }
        if bucket.is_empty() {
            index.remove(&key);
        }
    }
}

/// [`remove_pairs`] for a two-level index: each `(outer, key, value)` of `triples`.
fn remove_nested(nested: &mut FxHashMap<Id, Index>, mut triples: Vec<(Id, Id, Id)>, drop_empty: bool) {
    triples.sort_unstable();
    for group in triples.chunk_by(|a, b| a.0 == b.0) {
        let outer = group[0].0;
        let Some(index) = nested.get_mut(&outer) else {
            continue;
        };
        let mut pairs: Vec<(Id, Id)> = group.iter().map(|&(_, k, x)| (k, x)).collect();
        remove_pairs(index, &mut pairs);
        if drop_empty && index.is_empty() {
            nested.remove(&outer);
        }
    }
}
