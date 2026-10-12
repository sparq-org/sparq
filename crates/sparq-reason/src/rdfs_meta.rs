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
//! With the `explain` feature each derived fact keeps its FIRST derivation (rule + the two
//! premises). Premises were facts before it, so the provenance is acyclic and replayable.

use crate::Vocab;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::Id;

type T = [Id; 3];
type Derivation = (&'static str, [T; 2]);

#[derive(Default)]
pub(crate) struct MetaClosure {
    /// Every fact in the closure (asserted and derived).
    pub(crate) facts: FxHashSet<T>,
    /// `x -> [a]` for every subClassOf / subPropertyOf fact `(a R x)` (closure or edge).
    sc_in: FxHashMap<Id, Vec<Id>>,
    sp_in: FxHashMap<Id, Vec<Id>>,
    /// `a -> [b]` for every raw edge `(a R b)`.
    sc_edges_out: FxHashMap<Id, Vec<Id>>,
    sp_edges_out: FxHashMap<Id, Vec<Id>>,
    dom: FxHashMap<Id, Vec<Id>>,
    rng: FxHashMap<Id, Vec<Id>>,
    /// `c -> [s]` for every `(s rdf:type c)`.
    type_sub: FxHashMap<Id, Vec<Id>>,
    /// `p -> [(s, o)]` for every fact.
    po: FxHashMap<Id, Vec<(Id, Id)>>,
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

    /// Add asserted triples; returns the facts that newly entered the closure (including any
    /// of `triples` that were not facts yet).
    pub(crate) fn insert(&mut self, triples: &[T], v: &Vocab) -> Vec<T> {
        let mut added = Vec::new();
        let mut work: Vec<(T, bool)> = Vec::new();
        for &t in triples {
            self.offer(t, true, None, &mut work, &mut added, v);
        }
        let mut out = Vec::new();
        while let Some((t, edge)) = work.pop() {
            self.process(t, edge, &mut out, v);
            for (c, e, why) in out.drain(..) {
                self.offer(c, e, Some(why), &mut work, &mut added, v);
            }
        }
        added
    }

    /// A first derivation of `t` enters the closure; `edge` (only meaningful for
    /// subClassOf / subPropertyOf) marks a raw edge. A fact first derived by transitivity and
    /// later re-derived as an edge needs no second pass (it already has an edge path).
    #[inline]
    fn offer(
        &mut self,
        t: T,
        edge: bool,
        _why: Option<Derivation>,
        work: &mut Vec<(T, bool)>,
        added: &mut Vec<T>,
        v: &Vocab,
    ) {
        if self.facts.insert(t) {
            #[cfg(feature = "explain")]
            if let Some(w) = _why {
                self.prov.insert(t, w);
            }
            added.push(t);
            work.push((t, edge && (t[1] == v.sub_class || t[1] == v.sub_prop)));
        }
    }

    fn process(&mut self, t: T, edge: bool, out: &mut Vec<(T, bool, Derivation)>, v: &Vocab) {
        let [s, p, o] = t;
        // 1. Index `t` in every role first, so the joins below also see `t` itself.
        self.po.entry(p).or_default().push((s, o));
        if p == v.sub_prop {
            self.sp_in.entry(o).or_default().push(s);
            if edge {
                self.sp_edges_out.entry(s).or_default().push(o);
            }
        } else if p == v.sub_class {
            self.sc_in.entry(o).or_default().push(s);
            if edge {
                self.sc_edges_out.entry(s).or_default().push(o);
            }
        } else if p == v.domain {
            self.dom.entry(s).or_default().push(o);
        } else if p == v.range {
            self.rng.entry(s).or_default().push(o);
        } else if p == v.ty {
            self.type_sub.entry(o).or_default().push(s);
        }
        // 2. `t` as the data premise (any predicate, the schema ones included).
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
        // 3. `t` as a schema premise.
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
            if edge {
                // Edge side: fact (x R s), edge (s R o) ⊢ (x R o).
                let ins = if p == v.sub_class {
                    &self.sc_in
                } else {
                    &self.sp_in
                };
                if let Some(xs) = ins.get(&s) {
                    out.extend(
                        xs.iter()
                            .map(|&x| ([x, p, o], false, (rule, [[x, p, s], t]))),
                    );
                }
                if p == v.sub_prop {
                    // rdfs7: (x s y), edge (s sp o) ⊢ (x o y).
                    if let Some(pairs) = self.po.get(&s) {
                        out.extend(
                            pairs
                                .iter()
                                .map(|&(x, y)| ([x, o, y], true, ("rdfs7", [t, [x, s, y]]))),
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
        } else if p == v.domain || p == v.range {
            let (rule, range) = if p == v.domain {
                ("rdfs2", false)
            } else {
                ("rdfs3", true)
            };
            if let Some(pairs) = self.po.get(&s) {
                out.extend(pairs.iter().map(|&(x, y)| {
                    (
                        [if range { y } else { x }, v.ty, o],
                        true,
                        (rule, [t, [x, s, y]]),
                    )
                }));
            }
        }
    }
}
