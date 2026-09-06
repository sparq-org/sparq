//! OWL 2 RL materialization (the forward-chainable OWL profile).
//!
//! Implements a correct, useful subset of the W3C OWL 2 RL/RDF rules
//! (<https://www.w3.org/TR/owl2-profiles/#Reasoning_in_OWL_2_RL_and_RDF_Graphs>) over the
//! same dictionary-encoded forward-chaining fixpoint as RDFS — RL rules are datalog-style
//! joins over integer ids. The RDFS rules (rdfs2/3/5/7/9/11) are included (RL subsumes them).
//!
//! | group | rule | premise ⊢ conclusion |
//! |---|---|---|
//! | equality | eq-sym   | `(x sameAs y)` ⊢ `(y sameAs x)` |
//! |          | eq-trans | `(x sameAs y),(y sameAs z)` ⊢ `(x sameAs z)` |
//! |          | eq-rep-s/p/o | `(s sameAs s'),(s p o)` ⊢ `(s' p o)` (and for p, o) |
//! | property | prp-inv1/2 | `(p inverseOf q),(x p y)` ⊢ `(y q x)` (and converse) |
//! |          | prp-symp | `(p a SymmetricProperty),(x p y)` ⊢ `(y p x)` |
//! |          | prp-trp  | `(p a TransitiveProperty),(x p y),(y p z)` ⊢ `(x p z)` |
//! |          | prp-eqp1/2 | `(p equivalentProperty q),(x p y)` ⊢ `(x q y)` (and converse) |
//! |          | prp-fp   | `(p a FunctionalProperty),(x p y1),(x p y2)` ⊢ `(y1 sameAs y2)` |
//! |          | prp-ifp  | `(p a InverseFunctionalProperty),(x1 p y),(x2 p y)` ⊢ `(x1 sameAs x2)` |
//! | class    | cax-eqc1/2 | `(c equivalentClass d),(x a c)` ⊢ `(x a d)` (and converse) |
//!
//! Coverage now includes the class-expression rules (cls-* for someValuesFrom/allValuesFrom/
//! hasValue/oneOf/intersection/union via `owl:Restriction` + RDF-list decoding), `prp-spo2`
//! (propertyChainAxiom), cardinality/`hasKey`, the schema (scm-*) rules incl. the
//! restriction-subsumption family (scm-hv/svf1/svf2/avf1/avf2), the premise-free
//! cls-thing/cls-nothing1 axioms (occurrence-guarded), and the consistency
//! clashes incl. cls-maxqc1/2 (see [`inconsistencies`]). `owl:sameAs` is handled by union-find ENTITY REWRITING
//! (reason over canonical representatives, expand at the end) rather than the quadratic eq-rep
//! substitution. The fixpoint is SEMI-NAIVE: the recursive rules (RDFS transitivity + prp-trp)
//! derive only from the previous round's new facts against incrementally-maintained indexes.

use crate::{RdfsIndex, Schema, Vocab};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};

/// Below this many delta facts / candidates the per-round work runs single-threaded
/// (rayon fan-out is not worth the overhead). Matches `rdfs::PAR_THRESHOLD`.
#[cfg(feature = "parallel")]
const PAR_THRESHOLD: usize = 4096;

const OWL: &str = "http://www.w3.org/2002/07/owl#";
pub(crate) const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

struct Owl {
    same_as: Id,
    inverse_of: Id,
    symmetric: Id,       // owl:SymmetricProperty
    transitive: Id,      // owl:TransitiveProperty
    equiv_prop: Id,      // owl:equivalentProperty
    equiv_class: Id,     // owl:equivalentClass
    functional: Id,      // owl:FunctionalProperty
    inv_functional: Id,  // owl:InverseFunctionalProperty
    property_chain: Id,  // owl:propertyChainAxiom
    on_property: Id,     // owl:onProperty
    some_values: Id,     // owl:someValuesFrom
    all_values: Id,      // owl:allValuesFrom
    has_value: Id,       // owl:hasValue
    intersection: Id,    // owl:intersectionOf
    union: Id,           // owl:unionOf
    thing: Id,           // owl:Thing
    has_key: Id,         // owl:hasKey
    max_cardinality: Id, // owl:maxCardinality
    max_qual_card: Id,   // owl:maxQualifiedCardinality
    on_class: Id,        // owl:onClass
    one_of: Id,          // owl:oneOf
    rdf_first: Id,
    rdf_rest: Id,
    rdf_nil: Id,
}

impl Owl {
    fn intern(dict: &mut Dict) -> Owl {
        let mut i = |frag: &str| dict.intern_iri(&format!("{OWL}{frag}"));
        Owl {
            same_as: i("sameAs"),
            inverse_of: i("inverseOf"),
            symmetric: i("SymmetricProperty"),
            transitive: i("TransitiveProperty"),
            equiv_prop: i("equivalentProperty"),
            equiv_class: i("equivalentClass"),
            functional: i("FunctionalProperty"),
            inv_functional: i("InverseFunctionalProperty"),
            property_chain: i("propertyChainAxiom"),
            on_property: i("onProperty"),
            some_values: i("someValuesFrom"),
            all_values: i("allValuesFrom"),
            has_value: i("hasValue"),
            intersection: i("intersectionOf"),
            union: i("unionOf"),
            thing: i("Thing"),
            has_key: i("hasKey"),
            max_cardinality: i("maxCardinality"),
            max_qual_card: i("maxQualifiedCardinality"),
            on_class: i("onClass"),
            one_of: i("oneOf"),
            rdf_first: dict.intern_iri(&format!("{RDF}first")),
            rdf_rest: dict.intern_iri(&format!("{RDF}rest")),
            rdf_nil: dict.intern_iri(&format!("{RDF}nil")),
        }
    }
}

/// The integer value of a literal id (inline ints are direct; else parse the lexical form).
/// Used for `owl:maxCardinality` thresholds.
fn lit_int(dict: &Dict, id: Id) -> Option<i64> {
    if sparq_core::dict::is_inline(id) {
        return Some((id - sparq_core::dict::INLINE_BASE) as i64);
    }
    match dict.term(id) {
        oxrdf::Term::Literal(l) => l.value().parse().ok(),
        _ => None,
    }
}

/// Union-find over term ids for owl:sameAs ENTITY REWRITING (RDFox's approach). Instead of the
/// quadratic eq-rep substitution rule (copy every triple between every pair of equal terms,
/// re-derived each round), equal individuals are merged to a canonical representative (the
/// smallest id) and the whole closure is computed over representatives. The full eq-rep
/// expansion + the sameAs relation are emitted ONCE at the end. Large speed/memory win on
/// equality-heavy data; ~free when there are no equalities. (RDFox: up to 7.8× memory / 31×
/// time / 45–85× fewer derivations.)
#[derive(Default)]
struct UnionFind {
    parent: FxHashMap<Id, Id>,
}
impl UnionFind {
    fn find(&mut self, x: Id) -> Id {
        let mut root = x;
        while let Some(&p) = self.parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        // path compression
        let mut cur = x;
        while cur != root {
            let nxt = self.parent[&cur];
            self.parent.insert(cur, root);
            cur = nxt;
        }
        root
    }
    /// Read-only root lookup (no path compression) — usable from parallel workers. Returns
    /// the same representative as [`find`](Self::find): compression never changes roots.
    #[cfg(feature = "parallel")]
    fn find_ro(&self, x: Id) -> Id {
        let mut root = x;
        while let Some(&p) = self.parent.get(&root) {
            if p == root {
                break;
            }
            root = p;
        }
        root
    }
    /// Merge `a` and `b` (canonical rep = the smaller id). Returns true if newly merged.
    fn union(&mut self, a: Id, b: Id) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        let (lo, hi) = (ra.min(rb), ra.max(rb));
        self.parent.insert(hi, lo);
        self.parent.entry(lo).or_insert(lo);
        true
    }
    /// rep -> all member ids, for every non-singleton equivalence class.
    fn classes(&mut self) -> FxHashMap<Id, Vec<Id>> {
        let ids: Vec<Id> = self.parent.keys().copied().collect();
        let mut m: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
        for id in ids {
            let r = self.find(id);
            m.entry(r).or_default().push(id);
        }
        m
    }
}

/// Re-key every triple to its sameAs representative, dropping sameAs triples (the union-find
/// holds equality). Skips reflexive results.
fn canonicalize(all: &FxHashSet<[Id; 3]>, uf: &mut UnionFind, same_as: Id) -> FxHashSet<[Id; 3]> {
    let mut out = FxHashSet::default();
    for &[s, p, o] in all {
        if p == same_as {
            continue;
        }
        out.insert([uf.find(s), uf.find(p), uf.find(o)]);
    }
    out
}

/// (Re)build the transitive/functional/IFP adjacency over `all`: `out` maps p -> (subj -> [obj])
/// and `inc` maps p -> (obj -> [subj]), for the predicates in `need`. Used to seed the index
/// before the fixpoint and to rebuild it after a sameAs merge rewrites ids.
fn build_adjacency(
    all: &FxHashSet<[Id; 3]>,
    need: &FxHashSet<Id>,
    out: &mut FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
    inc: &mut FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
) {
    out.clear();
    inc.clear();
    if need.is_empty() {
        return;
    }
    for &[s, p, obj] in all {
        if need.contains(&p) {
            out.entry(p).or_default().entry(s).or_default().push(obj);
            inc.entry(p).or_default().entry(obj).or_default().push(s);
        }
    }
}

/// GENERATOR edges for the LINEAR (right-recursive) prp-trp evaluation: for each transitive
/// property `p`, the edges NOT derived by prp-trp itself (input edges + edges derived by any
/// other rule). The transitive closure of the generators equals the closure of the full
/// relation, so prp-trp can be evaluated as the LINEAR rule `R(x,y), GEN(y,z) ⊢ R(x,z)`
/// instead of the nonlinear `R ⋈ R`: each closure pair is then derived O(in-degree) times
/// (O(N²) total work on an N-chain) instead of once per intermediate node (O(N³)). Marking a
/// fact as generator is always SOUND (generators ⊆ R, so TC(gen) ⊆ R) and marking is COMPLETE
/// as long as every fact without a prp-trp derivation is included (then R ⊆ TC(gen)); facts
/// derived by both prp-trp and another rule may be marked either way.
#[derive(Default)]
struct TrpGen {
    /// p -> (subject -> [objects]) over generator edges only (the forward-join index).
    out: FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
    /// Generator membership (drives the backward `full ⋈ Δgen` join direction).
    set: FxHashSet<[Id; 3]>,
}
impl TrpGen {
    fn mark(&mut self, [s, p, o]: [Id; 3]) {
        if self.set.insert([s, p, o]) {
            self.out.entry(p).or_default().entry(s).or_default().push(o);
        }
    }
    /// Conservative rebuild (used at seed time and after a sameAs merge rewrites ids): every
    /// transitive-property edge in `all` becomes a generator. Correct — TC(R) = R — merely
    /// redundant for edges that were prp-trp-derived.
    fn rebuild(&mut self, all: &FxHashSet<[Id; 3]>, transitive: &FxHashSet<Id>) {
        self.out.clear();
        self.set.clear();
        if transitive.is_empty() {
            return;
        }
        for &t in all {
            if transitive.contains(&t[1]) {
                self.mark(t);
            }
        }
    }
}

/// Commit one round's candidates: a derived `sameAs` merges in the union-find (never stored);
/// anything else is canonicalized and inserted into `all` (+ the incremental `rdfs_idx` and
/// `out`/`inc` adjacency), with genuinely-new facts pushed to `new_delta`. Returns whether any
/// sameAs MERGE happened. This is the serial commit (also the small-round / no-rayon path).
#[allow(clippy::too_many_arguments)]
fn commit_serial(
    cand: Vec<[Id; 3]>,
    same_as: Id,
    v: &Vocab,
    uf: &mut UnionFind,
    all: &mut FxHashSet<[Id; 3]>,
    rdfs_idx: &mut RdfsIndex,
    need: &FxHashSet<Id>,
    out: &mut FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
    inc: &mut FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
    new_delta: &mut Vec<[Id; 3]>,
    // prp-trp generator marking: `Some((gen, transitive))` for candidate batches NOT produced
    // by prp-trp (their new transitive-property edges are generators); `None` for the prp-trp
    // batch (its facts are TC-paths of existing generators — never generators themselves).
    gen_mark: Option<(&mut TrpGen, &FxHashSet<Id>)>,
) -> bool {
    let mut merged = false;
    let mut gen_mark = gen_mark;
    for t in cand {
        if t[1] == same_as {
            // A derived sameAs (prp-fp/ifp, cls-maxc2/maxqc, …) → merge, don't store.
            if uf.union(t[0], t[2]) {
                merged = true;
            }
        } else {
            let c = [uf.find(t[0]), uf.find(t[1]), uf.find(t[2])];
            if all.insert(c) {
                rdfs_idx.insert(c, v);
                if need.contains(&c[1]) {
                    out.entry(c[1]).or_default().entry(c[0]).or_default().push(c[2]);
                    inc.entry(c[1]).or_default().entry(c[2]).or_default().push(c[0]);
                }
                if let Some((gen, transitive)) = gen_mark.as_mut() {
                    if transitive.contains(&c[1]) {
                        gen.mark(c);
                    }
                }
                new_delta.push(c);
            }
        }
    }
    merged
}

/// Parallel one-shot commit for large rounds, consuming the serial-rule candidates (`cand`)
/// plus the per-chunk Vecs from the parallel generation sweep (never concatenated). Phases:
/// 1. extract + apply the sameAs merges (union-find mutation stays serial);
/// 2. canonicalize + membership-prefilter EVERYTHING in PARALLEL (read-only `find_ro` /
///    `all.contains`) — this drops the enormous duplicate mass of the Δ⋈full ∪ full⋈Δ joins
///    against `all` BEFORE any sort, so only candidates of genuinely-new facts survive;
/// 3. par-sort + dedup the survivors (small), then the serial insert pass.
///
/// Equivalent to [`commit_serial`]: candidate ORDER is immaterial (the union-find partition is
/// order-independent with min-id representatives, and when any merge happens the caller
/// recanonicalizes `all` and reruns a full round, absorbing any pre-merge canonicalization).
#[cfg(feature = "parallel")]
#[allow(clippy::too_many_arguments)]
fn commit_candidates(
    cand: Vec<[Id; 3]>,
    chunks: Vec<Vec<[Id; 3]>>,
    trp_cand: Vec<[Id; 3]>,
    trp_chunks: Vec<Vec<[Id; 3]>>,
    same_as: Id,
    v: &Vocab,
    uf: &mut UnionFind,
    all: &mut FxHashSet<[Id; 3]>,
    rdfs_idx: &mut RdfsIndex,
    need: &FxHashSet<Id>,
    out: &mut FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
    inc: &mut FxHashMap<Id, FxHashMap<Id, Vec<Id>>>,
    new_delta: &mut Vec<[Id; 3]>,
    gen: &mut TrpGen,
    transitive: &FxHashSet<Id>,
) -> bool {
    use rayon::prelude::*;
    let total = cand.len()
        + chunks.iter().map(Vec::len).sum::<usize>()
        + trp_cand.len()
        + trp_chunks.iter().map(Vec::len).sum::<usize>();
    if total < PAR_THRESHOLD {
        let mut merged = commit_serial(
            cand, same_as, v, uf, all, rdfs_idx, need, out, inc, new_delta,
            Some((gen, transitive)),
        );
        for ch in chunks {
            merged |= commit_serial(
                ch, same_as, v, uf, all, rdfs_idx, need, out, inc, new_delta,
                Some((gen, transitive)),
            );
        }
        merged |= commit_serial(
            trp_cand, same_as, v, uf, all, rdfs_idx, need, out, inc, new_delta, None,
        );
        for ch in trp_chunks {
            merged |= commit_serial(
                ch, same_as, v, uf, all, rdfs_idx, need, out, inc, new_delta, None,
            );
        }
        return merged;
    }
    let prof = std::env::var("SPARQ_OWL_PROF").is_ok(); // TEMP PROFILING (removed before merge)
    let t0 = std::time::Instant::now();
    let main_parts: Vec<&Vec<[Id; 3]>> = std::iter::once(&cand).chain(chunks.iter()).collect();
    let trp_parts: Vec<&Vec<[Id; 3]>> =
        std::iter::once(&trp_cand).chain(trp_chunks.iter()).collect();
    // 1. sameAs merges first (serial union-find; parallel extraction). prp-trp never derives
    //    sameAs but the scan is uniform (and cheap) over both candidate streams.
    let same: Vec<(Id, Id)> = main_parts
        .par_iter()
        .chain(trp_parts.par_iter())
        .flat_map_iter(|ch| ch.iter().filter(|t| t[1] == same_as).map(|t| (t[0], t[2])))
        .collect();
    let mut merged = false;
    for (a, b) in same {
        if uf.union(a, b) {
            merged = true;
        }
    }
    let t1 = std::time::Instant::now();
    // 2. canonicalize + prefilter against `all` in parallel — separately per stream, so the
    //    insert pass below knows which new facts are prp-trp-derived (→ not generators).
    let prefilter = |parts: &[&Vec<[Id; 3]>]| -> Vec<[Id; 3]> {
        let mut surv: Vec<[Id; 3]> = parts
            .par_iter()
            .flat_map_iter(|ch| {
                ch.iter()
                    .filter(|t| t[1] != same_as)
                    .map(|&[s, p, o]| [uf.find_ro(s), uf.find_ro(p), uf.find_ro(o)])
                    .filter(|c| !all.contains(c))
            })
            .collect();
        // 3. one-shot dedup of the survivors (parallel sort).
        surv.par_sort_unstable();
        surv.dedup();
        surv
    };
    let surv_main = prefilter(&main_parts);
    let surv_trp = prefilter(&trp_parts);
    let t3 = std::time::Instant::now();
    let ns = surv_main.len() + surv_trp.len();
    // 4. serial insert pass. A fact surviving in BOTH streams keeps whichever marking inserts
    //    first — facts with a prp-trp derivation may be generator or not, both are correct
    //    (see [`TrpGen`]); only facts with NO prp-trp derivation must be marked, and those
    //    only ever appear in `surv_main`.
    for (surv, is_trp) in [(surv_main, false), (surv_trp, true)] {
        for c in surv {
            if all.insert(c) {
                rdfs_idx.insert(c, v);
                if need.contains(&c[1]) {
                    out.entry(c[1]).or_default().entry(c[0]).or_default().push(c[2]);
                    inc.entry(c[1]).or_default().entry(c[2]).or_default().push(c[0]);
                }
                if !is_trp && transitive.contains(&c[1]) {
                    gen.mark(c);
                }
                new_delta.push(c);
            }
        }
    }
    if prof {
        eprintln!(
            "OWL-PROF-COMMIT cand={total} surv={ns} union={:.3} prefilter+sort={:.3} insert={:.3}",
            (t1 - t0).as_secs_f64(),
            (t3 - t1).as_secs_f64(),
            t3.elapsed().as_secs_f64()
        );
    }
    merged
}

/// Indexes for the list/restriction/cardinality/key rules, maintained INCREMENTALLY across
/// fixpoint rounds (each round inserts just `delta`; cleared and reseeded after a sameAs merge
/// rewrites ids) instead of being rebuilt from `all` every round — that per-round O(|all|)
/// rebuild dominated on restriction-heavy ontologies. Contents mirror exactly what the old
/// per-round scan built: the structural TBox maps, `by_pred`/`type_subj`/`subj_types`, and the
/// decoded RDF lists (re-decoded only when a `rdf:first`/`rdf:rest`/list-head triple arrives).
#[derive(Default)]
struct ClassFeatureIdx {
    first: FxHashMap<Id, Id>, // rdf:first / rdf:rest edges (for list decoding)
    rest: FxHashMap<Id, Id>,
    on_prop: FxHashMap<Id, Id>,    // restriction -> onProperty
    svf: FxHashMap<Id, Id>,        // restriction -> someValuesFrom class
    avf: FxHashMap<Id, Id>,        // restriction -> allValuesFrom class
    hv: FxHashMap<Id, Id>,         // restriction -> hasValue individual
    on_class: FxHashMap<Id, Id>,   // restriction -> onClass
    max_card: FxHashMap<Id, i64>,  // restriction -> maxCardinality
    max_qcard: FxHashMap<Id, i64>, // restriction -> maxQualifiedCardinality
    key_head: FxHashMap<Id, Id>,   // class -> hasKey list head
    chain_head: FxHashMap<Id, Id>, // property -> propertyChainAxiom list head
    inter_head: FxHashMap<Id, Id>, // class -> intersectionOf list head
    union_head: FxHashMap<Id, Id>, // class -> unionOf list head
    oneof_head: FxHashMap<Id, Id>, // class -> oneOf list head
    by_pred: FxHashMap<Id, FxHashMap<Id, Vec<Id>>>, // p -> (s -> [o])
    type_subj: FxHashMap<Id, Vec<Id>>, // class -> subjects
    subj_types: FxHashMap<Id, Vec<Id>>, // subject -> classes
    lists_dirty: bool,
    lists: FxHashMap<Id, Vec<Id>>, // decoded list head -> members
    keys: FxHashMap<Id, Vec<Id>>,  // class -> hasKey property list
    chains: FxHashMap<Id, Vec<Id>>, // property -> chain property list
    inters: FxHashMap<Id, Vec<Id>>, // class -> intersection members
    unions: FxHashMap<Id, Vec<Id>>, // class -> union members
    oneofs: FxHashMap<Id, Vec<Id>>, // class -> oneOf (enumerated individual) members
}

impl ClassFeatureIdx {
    fn insert(&mut self, [s, p, obj]: [Id; 3], v: &Vocab, o: &Owl, dict: &Dict) {
        self.by_pred.entry(p).or_default().entry(s).or_default().push(obj);
        if p == v.ty {
            self.type_subj.entry(obj).or_default().push(s);
            self.subj_types.entry(s).or_default().push(obj);
        } else if p == o.on_property {
            self.on_prop.insert(s, obj);
        } else if p == o.some_values {
            self.svf.insert(s, obj);
        } else if p == o.all_values {
            self.avf.insert(s, obj);
        } else if p == o.has_value {
            self.hv.insert(s, obj);
        } else if p == o.max_cardinality {
            if let Some(n) = lit_int(dict, obj) {
                self.max_card.insert(s, n);
            }
        } else if p == o.max_qual_card {
            if let Some(n) = lit_int(dict, obj) {
                self.max_qcard.insert(s, n);
            }
        } else if p == o.on_class {
            self.on_class.insert(s, obj);
        } else if p == o.has_key {
            self.key_head.insert(s, obj);
            self.lists_dirty = true;
        } else if p == o.property_chain {
            self.chain_head.insert(s, obj);
            self.lists_dirty = true;
        } else if p == o.intersection {
            self.inter_head.insert(s, obj);
            self.lists_dirty = true;
        } else if p == o.union {
            self.union_head.insert(s, obj);
            self.lists_dirty = true;
        } else if p == o.one_of {
            self.oneof_head.insert(s, obj);
            self.lists_dirty = true;
        } else if p == o.rdf_first {
            self.first.insert(s, obj);
            self.lists_dirty = true;
        } else if p == o.rdf_rest {
            self.rest.insert(s, obj);
            self.lists_dirty = true;
        }
    }

    /// Re-decode the RDF lists and the list-valued feature maps (hasKey / propertyChainAxiom /
    /// intersectionOf / unionOf) — only when a relevant triple arrived since the last round
    /// (lists are pure TBox structure, so in practice this runs once).
    fn refresh_lists(&mut self, o: &Owl) {
        if !self.lists_dirty {
            return;
        }
        self.lists_dirty = false;
        self.lists.clear();
        for &head in self.first.keys() {
            let mut members = Vec::new();
            let mut cur = head;
            for _ in 0..self.first.len() + 1 {
                match self.first.get(&cur) {
                    Some(&m) => members.push(m),
                    None => break,
                }
                match self.rest.get(&cur) {
                    Some(&n) if n != o.rdf_nil => cur = n,
                    _ => break,
                }
            }
            self.lists.insert(head, members);
        }
        for (dst, heads) in [
            (&mut self.keys, &self.key_head),
            (&mut self.chains, &self.chain_head),
            (&mut self.inters, &self.inter_head),
            (&mut self.unions, &self.union_head),
            (&mut self.oneofs, &self.oneof_head),
        ] {
            dst.clear();
            for (&s, head) in heads {
                if let Some(l) = self.lists.get(head) {
                    dst.insert(s, l.clone());
                }
            }
        }
    }
}

/// [OPUS-4.8] Conservative detection of OWL feature predicates/classes that are reachable through
/// RDFS *before* any feature is asserted directly. The fast paths bypass the OWL fixpoint when no
/// OWL feature is present, but an OWL feature can be *entailed* by RDFS first:
///   - rdfs7:  `(:p rdfs:subPropertyOf owl:sameAs), (:a :p :b)` ⊢ `(:a owl:sameAs :b)` — a feature
///     predicate appears only after RDFS subproperty propagation.
///   - rdfs9:  `(:C rdfs:subClassOf owl:SymmetricProperty), (:p a :C)` ⊢ `(:p a owl:SymmetricProperty)`
///     — a feature class appears only after RDFS subclass propagation.
///
/// In both cases the single-pass fast path would emit the derived `owl:sameAs`/feature-type triple
/// as an *ordinary* triple WITHOUT running the equality/property reasoning it implies.
///
/// To stay sound we treat a feature as "used" if any feature predicate is a (transitive)
/// rdfs:subPropertyOf-superproperty of *some* predicate that actually occurs in the data, or any
/// feature class is a (transitive) rdfs:subClassOf-superclass of *some* type that actually occurs
/// on a node. This is conservative (it may force the fixpoint when the derivation is vacuous),
/// which is the safe direction.
///
/// Returns `(feature_pred_reachable, feature_type_reachable)`.
fn rdfs_reachable_features(
    triples: &[[Id; 3]],
    v: &Vocab,
    feature_preds: &FxHashSet<Id>,
    feature_types: &FxHashSet<Id>,
) -> (bool, bool) {
    // Build the subPropertyOf / subClassOf adjacency and collect predicates/types in use.
    let mut sub_prop: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut sub_class: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
    let mut preds_in_use: FxHashSet<Id> = FxHashSet::default();
    let mut types_in_use: FxHashSet<Id> = FxHashSet::default();
    for &[s, p, ob] in triples {
        if p == v.sub_prop {
            sub_prop.entry(s).or_default().push(ob);
        } else if p == v.sub_class {
            sub_class.entry(s).or_default().push(ob);
        }
        // Every predicate is a candidate rdfs7 source; every type-object a candidate rdfs9 source.
        preds_in_use.insert(p);
        if p == v.ty {
            types_in_use.insert(ob);
        }
    }
    // Does any in-use predicate reach a feature predicate via subPropertyOf+ (at least one hop)?
    // The non-reflexive `reaches` means a directly-asserted feature predicate (which the direct
    // scan in the callers already handles) does NOT count here — only a feature predicate that is
    // a *proper* RDFS superproperty of an in-use predicate triggers the fixpoint fallback.
    let pred_hit = preds_in_use.iter().any(|&start| reaches(start, &sub_prop, feature_preds));
    // Same for feature classes reached through subClassOf+.
    let type_hit = types_in_use.iter().any(|&start| reaches(start, &sub_class, feature_types));
    (pred_hit, type_hit)
}

/// [OPUS-4.8] Strictly-transitive (≥1 hop) reachability of any node in `targets` from `start` over
/// `adj`. The start node itself is NOT a hit even if it is a target — a feature reached only by
/// being directly used is not "RDFS-derived". This is deliberately non-reflexive.
fn reaches(start: Id, adj: &FxHashMap<Id, Vec<Id>>, targets: &FxHashSet<Id>) -> bool {
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut stack = vec![start];
    seen.insert(start);
    while let Some(cur) = stack.pop() {
        if let Some(sups) = adj.get(&cur) {
            for &nxt in sups {
                if targets.contains(&nxt) {
                    return true;
                }
                if seen.insert(nxt) {
                    stack.push(nxt);
                }
            }
        }
    }
    false
}

/// True if the ontology uses any OWL-specific feature (so it needs the OWL fixpoint). When false,
/// the OWL-RL closure is exactly RDFS + scm-dom/rng, computed by the fast single-pass path.
fn owl_uses_features(triples: &[[Id; 3]], v: &Vocab, o: &Owl) -> bool {
    let preds: FxHashSet<Id> = [
        o.same_as,
        o.inverse_of,
        o.equiv_prop,
        o.equiv_class,
        o.property_chain,
        o.on_property,
        o.some_values,
        o.all_values,
        o.has_value,
        o.intersection,
        o.union,
        o.has_key,
        o.max_cardinality,
        o.max_qual_card,
        o.on_class,
        o.one_of,
    ]
    .into_iter()
    .collect();
    let types: FxHashSet<Id> = [o.symmetric, o.transitive, o.functional, o.inv_functional].into_iter().collect();
    if triples.iter().any(|&[_, p, ob]| preds.contains(&p) || (p == v.ty && types.contains(&ob))) {
        return true;
    }
    // [OPUS-4.8] Also fall back to the fixpoint when an OWL feature predicate/class is reachable
    // through RDFS subPropertyOf/subClassOf (rdfs7/rdfs9) — the single-pass path would otherwise
    // emit the RDFS-derived feature triple without doing the equality/property reasoning. See 1402.
    let (pred_hit, type_hit) = rdfs_reachable_features(triples, v, &preds, &types);
    pred_hit || type_hit
}

/// If the ontology uses ONLY the monotone / non-recursive OWL-RL subset (equivalentClass,
/// equivalentProperty, inverseOf, SymmetricProperty) — and NONE of the recursive features that
/// require the fixpoint (owl:sameAs, Transitive/Functional/InverseFunctionalProperty,
/// propertyChainAxiom, the restriction/cardinality/hasKey/intersection/union family) — return a
/// [`MonoOwl`](crate::rdfs::MonoOwl) descriptor so the closure runs in the single-pass sweep.
/// Returns `None` (→ fixpoint) the moment any recursive feature is present.
fn monotone_only(triples: &[[Id; 3]], v: &Vocab, o: &Owl) -> Option<crate::rdfs::MonoOwl> {
    // Predicates / type-objects whose presence forces the recursive fixpoint.
    let recursive_preds: FxHashSet<Id> = [
        o.same_as,
        o.property_chain,
        o.on_property,
        o.some_values,
        o.all_values,
        o.has_value,
        o.intersection,
        o.union,
        o.has_key,
        o.max_cardinality,
        o.max_qual_card,
        o.on_class,
        o.one_of,
    ]
    .into_iter()
    .collect();
    let recursive_types: FxHashSet<Id> =
        [o.transitive, o.functional, o.inv_functional].into_iter().collect();

    // [OPUS-4.8] An OWL feature can also be introduced by RDFS entailment (rdfs7/rdfs9), e.g.
    // `(:p rdfs:subPropertyOf owl:sameAs)` (recursive) or `(:C rdfs:subClassOf owl:SymmetricProperty)`
    // (monotone). The mono descriptor below is built from DIRECTLY-asserted feature axioms only, so
    // an RDFS-reachable feature — recursive OR monotone — would be silently dropped. Force the full
    // fixpoint whenever any OWL feature predicate/class is RDFS-reachable. See 1402.
    let monotone_preds: FxHashSet<Id> =
        [o.inverse_of, o.equiv_class, o.equiv_prop].into_iter().collect();
    let monotone_types: FxHashSet<Id> = [o.symmetric].into_iter().collect();
    let all_feature_preds: FxHashSet<Id> =
        recursive_preds.union(&monotone_preds).copied().collect();
    let all_feature_types: FxHashSet<Id> =
        recursive_types.union(&monotone_types).copied().collect();
    let (pred_hit, type_hit) =
        rdfs_reachable_features(triples, v, &all_feature_preds, &all_feature_types);
    if pred_hit || type_hit {
        return None;
    }

    let mut mono = crate::rdfs::MonoOwl::default();
    for &[s, p, ob] in triples {
        if recursive_preds.contains(&p) {
            return None;
        }
        if p == o.inverse_of {
            mono.inverse.entry(s).or_default().push(ob);
            mono.inverse.entry(ob).or_default().push(s);
        } else if p == o.equiv_class {
            mono.equiv_class.push((s, ob));
        } else if p == o.equiv_prop {
            mono.equiv_prop.push((s, ob));
        } else if p == v.ty {
            if recursive_types.contains(&ob) {
                return None;
            }
            if ob == o.symmetric {
                mono.symmetric.insert(s);
            }
        }
    }
    Some(mono)
}

/// Expand `triples` in place with the OWL 2 RL (+ RDFS) closure. Returns NEW triple count.
pub fn materialize_owl_rl(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    // Monotone single-shot layers around the core fixpoint:
    // - pre: `owl:differentFrom` symmetry + the XSD datatype hierarchy (subClassOf
    //   edges for the numeric tower of every datatype IRI that occurs), so the
    //   in-closure rules (rdfs9/11, scm-rng1/scm-dom1) see them;
    // - post: scm-eqc2 / scm-eqp2 (mutual subClassOf/subPropertyOf ⊢ equivalence) —
    //   sound to run once per closure pass because equivalence edges over
    //   already-mutual pairs feed no OWL/RDFS rule anything new (both subsumptions
    //   already hold). (They CAN feed reif-ctr — a reifier can name an equivalence
    //   triple — which is why the reify loop below re-runs it after each round.)
    let pre = pre_monotone(dict, triples);
    let mut added = owl_rl_closure(dict, triples);
    added += post_equivalences(dict, triples);
    // [Kern] Quoted-triple (RDF 1.2 reifier) rules — the full bridge (reif-dtr +
    // reif-ctr); see `reify_fixpoint` below and the `reify` module docs.
    #[cfg(feature = "quoted-triples")]
    {
        added += reify_fixpoint(dict, triples, crate::reify::ReifyMode::Bridge);
    }
    pre + added
}

/// [FABLE-5] sq-afun3 (second increment of kern/quoted-triple-infer): the OWL 2 RL
/// (+ RDFS) closure with the quoted-triple reify layer driven in an explicit
/// [`ReifyMode`](crate::reify::ReifyMode). [`materialize_owl_rl`] is exactly
/// `ReifyMode::Bridge`; `ReifyMode::DestructureOnly` is the STRICT-OPACITY variant —
/// reif-dtr recovers the classic vocabulary from as-written quotations, but reif-ctr
/// never runs, so inference never mints a triple term (in particular the documented
/// `owl:sameAs` bridge composition derives no variant quotation). Same in-place /
/// return-count / idempotency contract as [`materialize_owl_rl`]. The incremental
/// counterpart is `MaterializedOwlGraph::with_reify_mode`, whose Fallback
/// re-materialization runs the mode it was constructed with (third increment); plain
/// `MaterializedOwlGraph::new` is `ReifyMode::Bridge`, i.e. this function's
/// [`materialize_owl_rl`] default.
#[cfg(feature = "quoted-triples")]
pub fn materialize_owl_rl_reify(
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
    mode: crate::reify::ReifyMode,
) -> usize {
    let pre = pre_monotone(dict, triples);
    let mut added = owl_rl_closure(dict, triples);
    added += post_equivalences(dict, triples);
    added += reify_fixpoint(dict, triples, mode);
    pre + added
}

/// [Kern] Quoted-triple (RDF 1.2 reifier) rules — see the `reify` module for the
/// rule table, the finite-Herbrand-base restrictions, and the termination argument.
/// Behind the opt-in `quoted-triples` feature (the reif-dtr/reif-ctr bridge is a
/// deliberate, NON-normative entailment extension — off by default so plain
/// `Profile::OwlRl` closures never change for data that happens to use the classic
/// reification vocabulary). When ON, still occurrence-guarded (zero cost +
/// byte-identical closure for reify-free data) and checked AFTER the main closure
/// because RL rules can derive the trigger vocabulary. The loop ALTERNATES reify
/// steps with the RL closure: destructured components feed the RL rules, and
/// RL-derived triples can enable reif-ctr. Each round adds at least one triple over
/// a finite Herbrand base, so the alternation terminates. In
/// `ReifyMode::DestructureOnly` the same loop runs reif-dtr alone (a strict subset —
/// the occurrence gate stays a sound over-approximation, and the alternation is
/// still needed because RL rules can derive `rdf:reifies` assertions reif-dtr sees).
#[cfg(feature = "quoted-triples")]
fn reify_fixpoint(
    dict: &mut Dict,
    triples: &mut Vec<[Id; 3]>,
    mode: crate::reify::ReifyMode,
) -> usize {
    let mut added = 0;
    if crate::reify::occurs(dict, triples) {
        loop {
            let n = crate::reify::step(dict, triples, mode);
            if n == 0 {
                break;
            }
            added += n + owl_rl_closure(dict, triples) + post_equivalences(dict, triples);
        }
    }
    added
}

/// The XSD numeric-tower subsumptions (direct edges; rdfs11 closes them).
/// OWL 2 RDF-based semantics gives every datatype map these inclusions.
const XSD_HIERARCHY: &[(&str, &str)] = &[
    ("byte", "short"),
    ("short", "int"),
    ("int", "long"),
    ("long", "integer"),
    ("integer", "decimal"),
    ("unsignedByte", "unsignedShort"),
    ("unsignedShort", "unsignedInt"),
    ("unsignedInt", "unsignedLong"),
    ("unsignedLong", "nonNegativeInteger"),
    ("nonNegativeInteger", "integer"),
    ("positiveInteger", "nonNegativeInteger"),
    ("negativeInteger", "nonPositiveInteger"),
    ("nonPositiveInteger", "integer"),
];

/// Pre-closure monotone facts: symmetric `owl:differentFrom` edges (the relation
/// is symmetric in the OWL semantics; deriving the mirror once up front is
/// complete because nothing else derives differentFrom), the upward XSD
/// datatype-hierarchy chain for every XSD datatype IRI occurring in the data,
/// and the cls-thing / cls-nothing1 axioms (`owl:Thing/Nothing rdf:type owl:Class`)
/// for whichever of the two terms actually occurs (the rules are premise-free;
/// the occurrence guard keeps closures of data that never mentions Thing/Nothing
/// free of injected vocabulary, the same discipline as the XSD chain).
fn pre_monotone(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let occurring: FxHashSet<Id> = triples.iter().flat_map(|t| t.iter().copied()).collect();
    let mut add: Vec<[Id; 3]> = Vec::new();

    // differentFrom symmetry.
    let different_from = dict.intern_iri(&format!("{OWL}differentFrom"));
    for &[s, p, o] in triples.iter() {
        if p == different_from && !set.contains(&[o, p, s]) {
            add.push([o, p, s]);
        }
    }

    // cls-thing / cls-nothing1 (Profiles §4.3 Table 6, premise-free):
    // ⊢ T(owl:Thing, rdf:type, owl:Class) and ⊢ T(owl:Nothing, rdf:type, owl:Class),
    // emitted only when the term occurs in the data (see the doc comment).
    let ty = dict.intern_iri(oxrdf::vocab::rdf::TYPE.as_str());
    let owl_class = dict.intern_iri(&format!("{OWL}Class"));
    for frag in ["Thing", "Nothing"] {
        let id = dict.intern_iri(&format!("{OWL}{frag}"));
        if occurring.contains(&id) && !set.contains(&[id, ty, owl_class]) {
            add.push([id, ty, owl_class]);
        }
    }

    // XSD hierarchy: for each occurring datatype, add its upward chain.
    let sub_class = dict.intern_iri(oxrdf::vocab::rdfs::SUB_CLASS_OF.as_str());
    let mut chain: Vec<(Id, Id)> = Vec::new();
    for &(sub, sup) in XSD_HIERARCHY {
        // Cheap occurrence probe: only intern when the SUB side already occurs
        // (or was introduced as a sup earlier in this pass).
        let sub_id = dict.intern_iri(&format!("{XSD}{sub}"));
        if occurring.contains(&sub_id) || chain.iter().any(|&(_, b)| b == sub_id) {
            let sup_id = dict.intern_iri(&format!("{XSD}{sup}"));
            chain.push((sub_id, sup_id));
        }
    }
    // XSD_HIERARCHY is topologically ordered (subs before sups), so one pass
    // suffices to follow chains introduced within this pass.
    for (sub_id, sup_id) in chain {
        if !set.contains(&[sub_id, sub_class, sup_id]) {
            add.push([sub_id, sub_class, sup_id]);
        }
    }

    let mut added = 0;
    let mut seen = set;
    for t in add {
        if seen.insert(t) {
            triples.push(t);
            added += 1;
        }
    }
    added
}

/// Post-closure scm-eqc2 / scm-eqp2: mutual `rdfs:subClassOf` ⊢
/// `owl:equivalentClass` (and the property analogue), both orientations.
fn post_equivalences(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    let v = Vocab::intern(dict);
    let equiv_class = dict.intern_iri(&format!("{OWL}equivalentClass"));
    let equiv_prop = dict.intern_iri(&format!("{OWL}equivalentProperty"));
    let mut set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let mut sc: FxHashSet<(Id, Id)> = FxHashSet::default();
    let mut sp: FxHashSet<(Id, Id)> = FxHashSet::default();
    for &[s, p, o] in triples.iter() {
        if p == v.sub_class {
            sc.insert((s, o));
        } else if p == v.sub_prop {
            sp.insert((s, o));
        }
    }
    let mut added = 0;
    let emit = |pairs: &FxHashSet<(Id, Id)>,
                    eq: Id,
                    set: &mut FxHashSet<[Id; 3]>,
                    triples: &mut Vec<[Id; 3]>,
                    added: &mut usize| {
        for &(a, b) in pairs.iter() {
            if a != b && pairs.contains(&(b, a)) {
                for t in [[a, eq, b], [b, eq, a]] {
                    if set.insert(t) {
                        triples.push(t);
                        *added += 1;
                    }
                }
            }
        }
    };
    emit(&sc, equiv_class, &mut set, triples, &mut added);
    emit(&sp, equiv_prop, &mut set, triples, &mut added);
    added
}

/// The OWL 2 RL fixpoint core (see [`materialize_owl_rl`]).
fn owl_rl_closure(dict: &mut Dict, triples: &mut Vec<[Id; 3]>) -> usize {
    let v = Vocab::intern(dict);
    let o = Owl::intern(dict);

    // Fast path 1: an ontology with NO OWL-specific feature has an OWL-RL closure equal to the
    // RDFS closure plus the scm-dom/rng domain/range closure — so it runs through the single-pass
    // (parallel, no fixpoint) materializer instead of the OWL fixpoint. The common "OWL profile
    // but really just classes/properties/domains" case then reasons ~as fast as RDFS.
    if !owl_uses_features(triples, &v, &o) {
        return crate::rdfs::rdfs_closure(dict, triples, true, &crate::rdfs::MonoOwl::default());
    }

    // Fast path 2: the ontology uses ONLY the MONOTONE / non-recursive OWL-RL subset —
    // equivalentClass, equivalentProperty, inverseOf, SymmetricProperty — and NONE of the
    // recursive features (sameAs, Transitive/Functional/InverseFunctional, propertyChain,
    // restrictions, hasKey, cardinality, intersection/union). Then the OWL-RL closure is
    // saturable in the single ABox pass: equiv* fold into subClass/subProperty, and
    // inverse/symmetric are handled by the property-orientation closure (see `PropExpand`).
    // This is byte-identical to the fixpoint on this subset (asserted in tests) and skips the
    // semi-naive loop entirely.
    if let Some(mono) = monotone_only(triples, &v, &o) {
        return crate::rdfs::rdfs_closure(dict, triples, true, &mono);
    }

    let mut all: FxHashSet<[Id; 3]> = triples.iter().copied().collect();

    // owl:sameAs entity rewriting: seed the union-find from explicit sameAs, then reason over
    // canonical representatives (no quadratic eq-rep during the fixpoint).
    let mut uf = UnionFind::default();
    for &[s, p, obj] in &all {
        if p == o.same_as {
            uf.union(s, obj);
        }
    }
    all = canonicalize(&all, &mut uf, o.same_as);

    // SEMI-NAIVE driver for the RECURSIVE rules (RDFS transitivity rdfs5/9/11 + prp-trp): an
    // incremental index over `all` plus the previous round's newly-derived facts (`delta`).
    // The other (non-recursive) OWL rules below stay naive — they re-scan all of `all` each
    // round, which is correct because they saturate in O(1) rounds; only the recursive rules
    // need many rounds, and re-deriving the whole closure every round was the cost.
    let mut rdfs_idx = RdfsIndex::default();
    for &t in &all {
        rdfs_idx.insert(t, &v);
    }
    let mut delta: Vec<[Id; 3]> = all.iter().copied().collect();

    // The restriction / list / cardinality / key rules are defined entirely by TBox structural
    // predicates that no OWL-RL rule ever derives, so whether the ontology uses any of them is
    // fixed by the input. Detect it once; if absent we skip building those (expensive,
    // per-round, O(|all|)) indexes and running those rules entirely.
    let feature_preds: FxHashSet<Id> = [
        o.on_property,
        o.some_values,
        o.all_values,
        o.has_value,
        o.max_cardinality,
        o.max_qual_card,
        o.on_class,
        o.has_key,
        o.property_chain,
        o.intersection,
        o.union,
        o.one_of,
    ]
    .into_iter()
    .collect();
    let uses_class_features = all.iter().any(|[_, p, _]| feature_preds.contains(p));
    // Incremental class-feature indexes (populated from `delta` inside the loop when used).
    let mut cf = ClassFeatureIdx::default();

    // The property axioms (inverse/symmetric/transitive/functional/IFP/equivalent) are TBox and
    // never derived, so build them ONCE (rebuilt only after a sameAs merge rewrites ids). The
    // transitive/functional/IFP adjacency (`out`/`inc`) is likewise maintained INCREMENTALLY —
    // delta edges are appended as they are derived — instead of rebuilt from `all` every round,
    // which was the dominant per-round cost on recursive (transitive-closure) workloads.
    let mut ax = Axioms::build(&all, &v, &o);
    let mut need: FxHashSet<Id> =
        ax.transitive.iter().chain(ax.functional.iter()).chain(ax.inv_functional.iter()).copied().collect();
    let mut out: FxHashMap<Id, FxHashMap<Id, Vec<Id>>> = FxHashMap::default();
    let mut inc: FxHashMap<Id, FxHashMap<Id, Vec<Id>>> = FxHashMap::default();
    build_adjacency(&all, &need, &mut out, &mut inc);
    // [SONNET-4.6] sq-qonbz.2 — under `substrate-join`, the `out`/`inc` FxHashMap adjacency
    // probes are replaced by the persistent `DeltaTable`-backed `DeltaAdj`. The FxHashMaps
    // above are still updated by `commit_serial`/`commit_candidates` (they are maintained but
    // never read — the probes below go through `adj` instead). `adj` is built here from the
    // same seed triple set and kept in sync via `extend_one` from `new_delta` after each commit.
    #[cfg(feature = "substrate-join")]
    let mut adj = crate::owl_delta_adj::DeltaAdj::build(&all, &need);
    // prp-trp generator edges (linear transitive-closure evaluation — see [`TrpGen`]).
    let mut gen = TrpGen::default();
    gen.rebuild(&all, &ax.transitive);

    // The schema (subClassOf/subPropertyOf/domain/range view) is rebuilt — and the scm-dom/rng
    // rules re-fired — only on rounds whose delta actually contains a schema predicate (plus
    // round 1 and after merges). On long transitive-chain runs the rounds are O(chain length),
    // and an unconditional per-round O(|all|) schema scan would dominate.
    let mut schema = Schema::default();
    let mut schema_stale = true;

    // TEMP PROFILING (dev only, removed before merge): SPARQ_OWL_PROF=1 prints phase totals.
    let prof = std::env::var("SPARQ_OWL_PROF").is_ok();
    let (mut t_schema, mut t_gen, mut t_scm, mut t_fpifp, mut t_class, mut t_commit, mut t_merge) =
        (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    let mut rounds = 0usize;
    let now = std::time::Instant::now;

    loop {
        rounds += 1;
        let __t = now();
        let schema_dirty = schema_stale
            || delta.iter().any(|t| {
                let p = t[1];
                p == v.sub_class || p == v.sub_prop || p == v.domain || p == v.range
            });
        if schema_dirty {
            schema = Schema::build(&all, &v);
            schema_stale = false;
        }
        t_schema += __t.elapsed().as_secs_f64();
        let mut cand: Vec<[Id; 3]> = Vec::new();
        let mut trp_cand: Vec<[Id; 3]> = Vec::new();
        let __t = now();

        // Delta-driven rules, fused into ONE per-fact emitter so the sweep over `delta` can fan
        // out over rayon (every index it joins against is read-only here):
        //  - RDFS rules (RL includes them) — SEMI-NAIVE: derive only from `delta` against the
        //    incremental index (RdfsIndex::derive fires each rule in both delta directions),
        //    never re-scanning the whole closure.
        //  - Property/class-equivalence rules: single-premise over assertions joined against the
        //    fixed property axioms (axiom side never changes).
        //  - prp-fp/prp-ifp: delta-driven — a new edge pairs against the existing edges of the
        //    same subject (fp) / object (ifp) in the incremental adjacency. Each conflicting
        //    pair is emitted when its LATER edge arrives; both-new-this-round pairs are covered
        //    because `out`/`inc` already contain the current delta (appended at the previous
        //    commit). Replaces the old per-round FULL adjacency scan, which would dominate on
        //    long (many-round) fixpoints.
        //  - prp-trp: (x p y),(y p z) ⊢ (x p z) — LINEARIZED semi-naive (see [`TrpGen`]): a new
        //    fact extends forward ONLY through the generator edges `gen.out[y]` (not the whole
        //    relation), and a new GENERATOR edge extends every existing path ending at its
        //    start backward through the full `inc[x]`. Candidates go to the separate `trp`
        //    stream so the commit can mark generators. This is the chain-transitivity fix:
        //    the nonlinear Δ⋈full join derived each closure pair once per intermediate node
        //    (O(N³) candidates on an N-chain); the linear form derives it once per incoming
        //    generator edge (O(N²) total).
        let emit_delta = |[s, p, obj]: [Id; 3], cand: &mut Vec<[Id; 3]>, trp: &mut Vec<[Id; 3]>| {
            rdfs_idx.derive([s, p, obj], &v, cand);
            // (eq-sym / eq-rep are handled by the union-find, not as rules.)
            // --- property axioms on assertion (s p obj) ------------------------------
            if let Some(invs) = ax.inverse.get(&p) {
                cand.extend(invs.iter().map(|&q| [obj, q, s])); // prp-inv1/2
            }
            if ax.symmetric.contains(&p) {
                cand.push([obj, p, s]); // prp-symp
            }
            if let Some(eqp) = ax.equiv_prop.get(&p) {
                cand.extend(eqp.iter().map(|&q| [s, q, obj])); // prp-eqp1/2
            }
            // --- class equivalence on type assertion ---------------------------------
            if p == v.ty {
                if let Some(eqc) = ax.equiv_class.get(&obj) {
                    cand.extend(eqc.iter().map(|&d| [s, v.ty, d])); // cax-eqc1/2
                }
            }
            // --- scm-eqc/eqp: equivalence ⊢ subClassOf/subPropertyOf both ways ---------
            if p == o.equiv_class {
                cand.push([s, v.sub_class, obj]);
                cand.push([obj, v.sub_class, s]);
            } else if p == o.equiv_prop {
                cand.push([s, v.sub_prop, obj]);
                cand.push([obj, v.sub_prop, s]);
            }
            // --- prp-fp / prp-ifp (delta-driven; see the block comment above) --------
            if ax.functional.contains(&p) {
                // [SONNET-4.6] sq-qonbz.2: under `substrate-join` probe through DeltaAdj
                // (persistent build-side table, keyed on [p, s]); default: plain FxHashMap.
                #[cfg(not(feature = "substrate-join"))]
                if let Some(ys) = out.get(&p).and_then(|m| m.get(&s)) {
                    cand.extend(ys.iter().filter(|&&y| y != obj).map(|&y| [obj, o.same_as, y]));
                }
                #[cfg(feature = "substrate-join")]
                adj.probe_out(p, s, |y| {
                    if y != obj {
                        cand.push([obj, o.same_as, y]);
                    }
                });
            }
            if ax.inv_functional.contains(&p) {
                // [SONNET-4.6] sq-qonbz.2: under `substrate-join` probe through DeltaAdj
                // (persistent build-side table, keyed on [p, o]); default: plain FxHashMap.
                #[cfg(not(feature = "substrate-join"))]
                if let Some(xs) = inc.get(&p).and_then(|m| m.get(&obj)) {
                    cand.extend(xs.iter().filter(|&&x| x != s).map(|&x| [s, o.same_as, x]));
                }
                #[cfg(feature = "substrate-join")]
                adj.probe_inc(p, obj, |x| {
                    if x != s {
                        cand.push([s, o.same_as, x]);
                    }
                });
            }
            // --- prp-trp, linearized against the generator edges ---------------------
            if ax.transitive.contains(&p) {
                // forward: Δ ⋈ GEN — extend the new path by the generator edges at its end.
                if let Some(zs) = gen.out.get(&p).and_then(|m| m.get(&obj)) {
                    trp.extend(zs.iter().map(|&z| [s, p, z]));
                }
                // backward: full ⋈ Δgen — a new generator edge extends every existing path
                // ending at its start (incl. same-round delta paths, already in `inc`).
                if gen.set.contains(&[s, p, obj]) {
                    // [SONNET-4.6] sq-qonbz.2: probe DeltaAdj backward table under
                    // `substrate-join`; plain FxHashMap under default.
                    #[cfg(not(feature = "substrate-join"))]
                    if let Some(ws) = inc.get(&p).and_then(|m| m.get(&s)) {
                        trp.extend(ws.iter().map(|&w| [w, p, obj]));
                    }
                    #[cfg(feature = "substrate-join")]
                    adj.probe_inc(p, s, |w| trp.push([w, p, obj]));
                }
            }
        };
        // With rayon the sweep yields PER-CHUNK candidate Vecs that flow straight into the
        // parallel commit (never concatenated — at tens of millions of candidates per round the
        // reduce-tree memcpy is itself a major cost). Serial-rule candidates stay in `cand`.
        #[cfg(feature = "parallel")]
        let mut cand_chunks: Vec<Vec<[Id; 3]>> = Vec::new();
        #[cfg(feature = "parallel")]
        let mut trp_chunks: Vec<Vec<[Id; 3]>> = Vec::new();
        #[cfg(feature = "parallel")]
        if delta.len() >= PAR_THRESHOLD {
            use rayon::prelude::*;
            let chunk = (delta.len() / (rayon::current_num_threads() * 8)).max(1024);
            (cand_chunks, trp_chunks) = delta
                .par_chunks(chunk)
                .map(|ch| {
                    let mut acc = Vec::new();
                    let mut acc_trp = Vec::new();
                    for &t in ch {
                        emit_delta(t, &mut acc, &mut acc_trp);
                    }
                    (acc, acc_trp)
                })
                .unzip();
        } else {
            for &t in &delta {
                emit_delta(t, &mut cand, &mut trp_cand);
            }
        }
        #[cfg(not(feature = "parallel"))]
        for &t in &delta {
            emit_delta(t, &mut cand, &mut trp_cand);
        }
        t_gen += __t.elapsed().as_secs_f64();
        let __t = now();

        // --- scm-dom1/2, scm-rng1/2: domain/range propagate UP subClassOf and DOWN
        // subPropertyOf (makes the schema-level domain/range closure explicit). Re-fired only
        // when the schema changed this round — with an unchanged schema (and unchanged axioms;
        // those only change on merge, which forces `schema_dirty`) the block would re-emit
        // exactly the previous round's candidates, all duplicates. ----------
        if schema_dirty {
            let mut subprop_inv: FxHashMap<Id, Vec<Id>> = FxHashMap::default();
            for (&p2, supers) in &schema.sub_prop {
                for &sup in supers {
                    subprop_inv.entry(sup).or_default().push(p2);
                }
            }
            for (which, map) in [(v.domain, &schema.domain), (v.range, &schema.range)] {
                for (&p, classes) in map {
                    for &c in classes {
                        // scm-dom1/rng1: (p domain/range c), (c subClassOf d) ⊢ (p domain/range d).
                        if let Some(ds) = schema.sub_class.get(&c) {
                            cand.extend(ds.iter().map(|&d| [p, which, d]));
                        }
                    }
                    // scm-dom2/rng2: (p2 subPropertyOf p), (p domain/range c) ⊢ (p2 domain/range c).
                    if let Some(subs) = subprop_inv.get(&p) {
                        for &p2 in subs {
                            cand.extend(classes.iter().map(|&c| [p2, which, c]));
                        }
                    }
                    // inverse transposition: (p inverseOf q), (p domain c) ⊢ (q range c)
                    // and (p range c) ⊢ (q domain c) — a valid OWL (RDF-based)
                    // entailment the sparql11/entailment suite exercises.
                    if let Some(invs) = ax.inverse.get(&p) {
                        let other = if which == v.domain { v.range } else { v.domain };
                        for &q in invs {
                            cand.extend(classes.iter().map(|&c| [q, other, c]));
                        }
                    }
                }
            }
        }

        t_scm += __t.elapsed().as_secs_f64();
        let __t = now();
        // (prp-fp / prp-ifp / prp-trp are fused into the delta sweep above — all delta-driven
        // against the incrementally-maintained `out`/`inc`/`gen` adjacency.)
        t_fpifp += __t.elapsed().as_secs_f64();
        let __t = now();
        // --- list/restriction rules (prp-spo2, cls-svf/avf/hv, cls-int, scm-uni) ------
        // Decode RDF lists + restrictions + class lists once, plus the adjacency / type
        // indexes the rules join over. Skipped wholesale when the ontology uses none of these
        // features (the common case) — that is where the per-round O(|all|) cost was going.
        if uses_class_features {
            // Incremental: fold this round's delta into the persistent index (round 1 seeds it
            // with the whole input since `delta` starts as `all`; a sameAs merge clears it and
            // the post-merge full round reseeds it from the rewritten `all`).
            for &t in &delta {
                cf.insert(t, &v, &o, dict);
            }
            cf.refresh_lists(&o);
            let ClassFeatureIdx {
                on_prop,
                svf,
                avf,
                hv,
                max_card,
                max_qcard,
                on_class,
                keys,
                chains,
                inters,
                unions,
                oneofs,
                by_pred,
                type_subj,
                subj_types,
                ..
            } = &cf;
            let has_type = |x: Id, c: Id| subj_types.get(&x).is_some_and(|cs| cs.contains(&c));

            // cls-maxc2 — maxCardinality 1: the (≤1) values of `p` on an x∈R are all sameAs.
            for (&r, &n) in max_card {
                if n == 1 {
                    if let (Some(&p), Some(xs)) = (on_prop.get(&r), type_subj.get(&r)) {
                        if let Some(adj) = by_pred.get(&p) {
                            for &x in xs {
                                if let Some(ys) = adj.get(&x) {
                                    for i in 0..ys.len() {
                                        for j in (i + 1)..ys.len() {
                                            cand.push([ys[i], o.same_as, ys[j]]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // cls-maxqc3/4 — maxQualifiedCardinality 1 onClass c: the (≤1) c-typed values are sameAs.
            for (&r, &n) in max_qcard {
                if n == 1 {
                    if let (Some(&p), Some(&c), Some(xs)) =
                        (on_prop.get(&r), on_class.get(&r), type_subj.get(&r))
                    {
                        if let Some(adj) = by_pred.get(&p) {
                            for &x in xs {
                                if let Some(ys) = adj.get(&x) {
                                    let q: Vec<Id> = ys
                                        .iter()
                                        .copied()
                                        .filter(|&y| c == o.thing || has_type(y, c))
                                        .collect();
                                    for i in 0..q.len() {
                                        for j in (i + 1)..q.len() {
                                            cand.push([q[i], o.same_as, q[j]]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // prp-key — owl:hasKey: individuals of a class agreeing on all key-property values are
            // the same. (Single-value-per-key common case: group by the key-value tuple.)
            for (&c, kprops) in keys {
                if kprops.is_empty() {
                    continue;
                }
                if let Some(individuals) = type_subj.get(&c) {
                    let mut by_tuple: FxHashMap<Vec<Id>, Id> = FxHashMap::default();
                    for &x in individuals {
                        let mut tuple = Vec::with_capacity(kprops.len());
                        let mut complete = true;
                        for &kp in kprops {
                            match by_pred
                                .get(&kp)
                                .and_then(|adj| adj.get(&x))
                                .and_then(|vs| vs.first())
                            {
                                Some(&val) => tuple.push(val),
                                None => {
                                    complete = false;
                                    break;
                                }
                            }
                        }
                        if complete {
                            if let Some(&y) = by_tuple.get(&tuple) {
                                cand.push([x, o.same_as, y]);
                            } else {
                                by_tuple.insert(tuple, x);
                            }
                        }
                    }
                }
            }

            // prp-spo2 — property chain: (x p1 y1)…(y_{n-1} pn z) ⊢ (x p z).
            for (&p, chain) in chains {
                if chain.is_empty() {
                    continue;
                }
                let mut paths: Vec<(Id, Id)> = Vec::new();
                if let Some(adj) = by_pred.get(&chain[0]) {
                    for (&s, os) in adj {
                        paths.extend(os.iter().map(|&z| (s, z)));
                    }
                }
                for &pi in &chain[1..] {
                    let mut next = Vec::new();
                    if let Some(adj) = by_pred.get(&pi) {
                        for &(start, mid) in &paths {
                            if let Some(os) = adj.get(&mid) {
                                next.extend(os.iter().map(|&z| (start, z)));
                            }
                        }
                    }
                    paths = next;
                    if paths.is_empty() {
                        break;
                    }
                }
                cand.extend(paths.into_iter().map(|(x, z)| [x, p, z]));
            }

            // cls-svf1 — someValuesFrom: (x p u),(u type c) ⊢ (x type R)   [c = owl:Thing ⇒ any u]
            for (&r, &c) in svf {
                if let Some(&p) = on_prop.get(&r) {
                    if let Some(adj) = by_pred.get(&p) {
                        for (&x, us) in adj {
                            if us.iter().any(|&u| c == o.thing || has_type(u, c)) {
                                cand.push([x, v.ty, r]);
                            }
                        }
                    }
                }
            }
            // cls-avf1 — allValuesFrom: (x type R),(x p u) ⊢ (u type c)
            for (&r, &c) in avf {
                if let (Some(&p), Some(xs)) = (on_prop.get(&r), type_subj.get(&r)) {
                    if let Some(adj) = by_pred.get(&p) {
                        for &x in xs {
                            if let Some(us) = adj.get(&x) {
                                cand.extend(us.iter().map(|&u| [u, v.ty, c]));
                            }
                        }
                    }
                }
            }
            // cls-hv1/hv2 — hasValue: (x type R) ⊢ (x p w) and (x p w) ⊢ (x type R)
            for (&r, &w) in hv {
                if let Some(&p) = on_prop.get(&r) {
                    if let Some(xs) = type_subj.get(&r) {
                        cand.extend(xs.iter().map(|&x| [x, p, w])); // cls-hv1
                    }
                    if let Some(adj) = by_pred.get(&p) {
                        for (&x, os) in adj {
                            if os.contains(&w) {
                                cand.push([x, v.ty, r]); // cls-hv2
                            }
                        }
                    }
                }
            }
            // cls-int1/int2 — intersectionOf: x type all members ⇔ x type c.
            for (&c, members) in inters {
                if members.is_empty() {
                    continue;
                }
                if let Some(xs) = type_subj.get(&c) {
                    for &x in xs {
                        cand.extend(members.iter().map(|&m| [x, v.ty, m])); // int2
                    }
                }
                if let Some(first) = type_subj.get(&members[0]) {
                    for &x in first {
                        if members.iter().all(|&m| has_type(x, m)) {
                            cand.push([x, v.ty, c]); // int1
                        }
                    }
                }
            }
            // scm-uni — unionOf: x type member ⊢ x type c.
            for (&c, members) in unions {
                for &m in members {
                    if let Some(xs) = type_subj.get(&m) {
                        cand.extend(xs.iter().map(|&x| [x, v.ty, c]));
                    }
                }
            }
            // scm-int / scm-uni (schema level): intersectionOf c ⊢ c subClassOf each member;
            // unionOf c ⊢ each member subClassOf c. Makes the class hierarchy explicit (queryable),
            // complementing the type-level cls-int2/scm-uni rules above.
            for (&c, members) in inters {
                cand.extend(members.iter().map(|&m| [c, v.sub_class, m]));
            }
            for (&c, members) in unions {
                cand.extend(members.iter().map(|&m| [m, v.sub_class, c]));
            }
            // cls-oo — oneOf: `T(?c, owl:oneOf, ?x), LIST[?x, ?y1…?yn] ⊢ T(?yi, rdf:type, ?c)`
            // (every enumerated individual is an instance of the enumeration class). Linear in
            // the total oneOf list length (the lists are decoded once by `refresh_lists`).
            for (&c, members) in oneofs {
                cand.extend(members.iter().map(|&y| [y, v.ty, c]));
            }
            // scm-hv / scm-svf1/2 / scm-avf1/2 — schema-level restriction subsumption
            // (Profiles §4.3 Table 9):
            //   scm-svf1: (c1 svf y1; onProp p) (c2 svf y2; onProp p) (y1 ⊑c y2) ⊢ c1 ⊑c c2
            //   scm-avf1: same with allValuesFrom
            //   scm-svf2: (c1 svf y; onProp p1) (c2 svf y; onProp p2) (p1 ⊑p p2) ⊢ c1 ⊑c c2
            //   scm-hv:   (c1 hv i; onProp p1) (c2 hv i; onProp p2) (p1 ⊑p p2) ⊢ c1 ⊑c c2
            //   scm-avf2: (c1 avf y; onProp p1) (c2 avf y; onProp p2) (p1 ⊑p p2) ⊢ c2 ⊑c c1
            //              (NB the conclusion direction REVERSES — allValuesFrom is
            //              contravariant in the property).
            // INDEXED/GUARDED: the naive form is a quadratic restriction×restriction join.
            // Instead restrictions are grouped by onProperty (svf1/avf1) or by filler/value
            // (svf2/avf2/hv), and within a group only the explicit subClassOf/subPropertyOf
            // out-edges of each member are probed — O(restrictions × direct super-edges),
            // never all pairs. Conclusions (subClassOf between restriction nodes) re-enter
            // the fixpoint and feed rdfs9/11 the same way the other scm-* rules do.
            {
                // p -> (filler -> [restrictions]) for the same-property rules.
                let group_by_prop = |m: &FxHashMap<Id, Id>| {
                    let mut g: FxHashMap<Id, FxHashMap<Id, Vec<Id>>> = FxHashMap::default();
                    for (&r, &y) in m {
                        if let Some(&p) = on_prop.get(&r) {
                            g.entry(p).or_default().entry(y).or_default().push(r);
                        }
                    }
                    g
                };
                // filler -> (p -> [restrictions]) for the same-filler rules.
                let group_by_filler = |m: &FxHashMap<Id, Id>| {
                    let mut g: FxHashMap<Id, FxHashMap<Id, Vec<Id>>> = FxHashMap::default();
                    for (&r, &y) in m {
                        if let Some(&p) = on_prop.get(&r) {
                            g.entry(y).or_default().entry(p).or_default().push(r);
                        }
                    }
                    g
                };
                // scm-svf1 / scm-avf1: same property, subClassOf-related fillers.
                for m in [svf, avf] {
                    for by_filler in group_by_prop(m).into_values() {
                        for (y1, r1s) in &by_filler {
                            if let Some(sups) = schema.sub_class.get(y1) {
                                for y2 in sups {
                                    if let Some(r2s) = by_filler.get(y2) {
                                        for &r1 in r1s {
                                            cand.extend(
                                                r2s.iter().map(|&r2| [r1, v.sub_class, r2]),
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // scm-svf2 / scm-hv (⊢ c1 ⊑ c2) and scm-avf2 (⊢ c2 ⊑ c1): same filler/value,
                // subPropertyOf-related properties.
                for (m, fwd) in [(svf, true), (hv, true), (avf, false)] {
                    for by_p in group_by_filler(m).into_values() {
                        for (p1, r1s) in &by_p {
                            if let Some(sups) = schema.sub_prop.get(p1) {
                                for p2 in sups {
                                    if let Some(r2s) = by_p.get(p2) {
                                        for &r1 in r1s {
                                            cand.extend(r2s.iter().map(|&r2| {
                                                if fwd {
                                                    [r1, v.sub_class, r2]
                                                } else {
                                                    [r2, v.sub_class, r1]
                                                }
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } // uses_class_features
        t_class += __t.elapsed().as_secs_f64();
        let __t = now();

        let mut new_delta: Vec<[Id; 3]> = Vec::new();
        #[cfg(feature = "parallel")]
        let merged = commit_candidates(
            cand,
            cand_chunks,
            trp_cand,
            trp_chunks,
            o.same_as,
            &v,
            &mut uf,
            &mut all,
            &mut rdfs_idx,
            &need,
            &mut out,
            &mut inc,
            &mut new_delta,
            &mut gen,
            &ax.transitive,
        );
        #[cfg(not(feature = "parallel"))]
        let merged = {
            let mut m = commit_serial(
                cand,
                o.same_as,
                &v,
                &mut uf,
                &mut all,
                &mut rdfs_idx,
                &need,
                &mut out,
                &mut inc,
                &mut new_delta,
                Some((&mut gen, &ax.transitive)),
            );
            m |= commit_serial(
                trp_cand,
                o.same_as,
                &v,
                &mut uf,
                &mut all,
                &mut rdfs_idx,
                &need,
                &mut out,
                &mut inc,
                &mut new_delta,
                None,
            );
            m
        };
        t_commit += __t.elapsed().as_secs_f64();
        // [SONNET-4.6] sq-qonbz.2 — extend DeltaAdj with the newly-committed facts whose
        // predicate is in `need`. This mirrors the `or_default().push()` updates that
        // `commit_serial`/`commit_candidates` apply to `out`/`inc` above. Per-round cost is
        // O(|new_delta|) — the same asymptotic cost as the FxHashMap path.
        #[cfg(feature = "substrate-join")]
        for &[s, p, obj] in &new_delta {
            if need.contains(&p) {
                adj.extend_one(s, p, obj);
            }
        }
        let __t = now();
        if merged {
            // A merge rewrites representatives across `all`; recanonicalize, rebuild the
            // incremental indexes, and run a full (naive) round next so nothing is missed. Merges
            // are bounded by the individual count, so this fallback cannot loop indefinitely.
            all = canonicalize(&all, &mut uf, o.same_as);
            ax = Axioms::build(&all, &v, &o);
            need =
                ax.transitive.iter().chain(ax.functional.iter()).chain(ax.inv_functional.iter()).copied().collect();
            build_adjacency(&all, &need, &mut out, &mut inc);
            // [SONNET-4.6] sq-qonbz.2 — rebuild DeltaAdj from the recanonicalised triple set
            // (mirrors the `build_adjacency` call above; union-find merge rewrites ids, so the
            // whole index must be rebuilt from scratch — same epoch as the FxHashMap path).
            #[cfg(feature = "substrate-join")]
            adj.rebuild(&all, &need);
            gen.rebuild(&all, &ax.transitive);
            schema_stale = true;
            rdfs_idx = RdfsIndex::default();
            for &t in &all {
                rdfs_idx.insert(t, &v);
            }
            // The class-feature index is keyed by pre-merge ids: clear it; the full round below
            // (delta = the whole rewritten `all`) reseeds it.
            cf = ClassFeatureIdx::default();
            delta = all.iter().copied().collect();
            t_merge += __t.elapsed().as_secs_f64();
        } else if new_delta.is_empty() {
            break;
        } else {
            // [OPUS-4.8] A property/class axiom can be DERIVED during the fixpoint (e.g. rdfs7
            // makes `:p rdfs:subPropertyOf owl:sameAs` emit nothing but rdfs9 derives
            // `:p rdf:type owl:SymmetricProperty`, or an axiom predicate arrives via subPropertyOf).
            // The per-fact emitter joins assertions against the property axiom maps (ax.*), which
            // were built once from the seed and are otherwise treated as immutable. When the delta
            // introduces a NEW axiom we must rebuild `ax`/adjacency/generators and re-fire ALL
            // assertions so the new axiom applies to pre-existing facts. Mirrors the merge rebuild.
            // See review 1402.
            let axiom_added = new_delta.iter().any(|&[_, p, ob]| {
                p == o.inverse_of
                    || p == o.equiv_prop
                    || p == o.equiv_class
                    || (p == v.ty
                        && (ob == o.symmetric
                            || ob == o.transitive
                            || ob == o.functional
                            || ob == o.inv_functional))
            });
            if axiom_added {
                // Roll the new delta into `all` first (commit already inserted it into `all`),
                // then rebuild the axiom-derived structures and run a full naive round next.
                ax = Axioms::build(&all, &v, &o);
                need = ax
                    .transitive
                    .iter()
                    .chain(ax.functional.iter())
                    .chain(ax.inv_functional.iter())
                    .copied()
                    .collect();
                build_adjacency(&all, &need, &mut out, &mut inc);
                // [SONNET-4.6] sq-qonbz.2 — rebuild DeltaAdj when a new property axiom arrives
                // (mirrors the `build_adjacency` call above; same epoch as the FxHashMap path).
                #[cfg(feature = "substrate-join")]
                adj.rebuild(&all, &need);
                gen.rebuild(&all, &ax.transitive);
                cf = ClassFeatureIdx::default();
                schema_stale = true;
                delta = all.iter().copied().collect();
            } else {
                delta = new_delta;
            }
        }
    }
    if prof {
        eprintln!(
            "OWL-PROF rounds={rounds} schema={t_schema:.3} gen={t_gen:.3} scm={t_scm:.3} \
             fpifp={t_fpifp:.3} class={t_class:.3} commit={t_commit:.3} merge={t_merge:.3}"
        );
    }
    let __t = now();

    // EXPAND the owl:sameAs equivalence classes back into the full closure: every canonical
    // triple is re-emitted for each member combination, plus the full sameAs relation. (The
    // memory-efficient canonical form would need a sameAs-aware query engine; we expand so the
    // existing engine answers queries over any member correctly.) When there are NO equivalence
    // classes (no sameAs anywhere — common), `all` already IS the closure: skip the rebuild,
    // which is otherwise a full O(|closure|) hash-set reconstruction.
    let classes = uf.classes();
    let snapshot: Vec<[Id; 3]> = all.iter().copied().collect();
    drop(all);
    let expanded: Vec<[Id; 3]> = if classes.is_empty() {
        snapshot
    } else {
        // Every canonical position in `all` is a representative and equivalence classes are
        // disjoint, so the per-triple expansions never collide — a flat Vec is set-exact (the
        // final sort+dedup below enforces set semantics regardless).
        let expand_one = |&[s, p, ob]: &[Id; 3], out: &mut Vec<[Id; 3]>| {
            let sm = classes
                .get(&s)
                .map_or(std::slice::from_ref(&s), |v| v.as_slice());
            let pm = classes
                .get(&p)
                .map_or(std::slice::from_ref(&p), |v| v.as_slice());
            let om = classes
                .get(&ob)
                .map_or(std::slice::from_ref(&ob), |v| v.as_slice());
            if sm.len() == 1 && pm.len() == 1 && om.len() == 1 {
                out.push([s, p, ob]); // common case: no equalities involved
            } else {
                for &s2 in sm {
                    for &p2 in pm {
                        for &o2 in om {
                            out.push([s2, p2, o2]);
                        }
                    }
                }
            }
        };
        #[cfg(feature = "parallel")]
        let mut expanded: Vec<[Id; 3]> = if snapshot.len() >= PAR_THRESHOLD {
            use rayon::prelude::*;
            snapshot
                .par_iter()
                .fold(Vec::new, |mut acc, t| {
                    expand_one(t, &mut acc);
                    acc
                })
                .reduce(Vec::new, |mut a, mut b| {
                    a.append(&mut b);
                    a
                })
        } else {
            let mut acc = Vec::new();
            for t in &snapshot {
                expand_one(t, &mut acc);
            }
            acc
        };
        #[cfg(not(feature = "parallel"))]
        let mut expanded: Vec<[Id; 3]> = {
            let mut acc = Vec::new();
            for t in &snapshot {
                expand_one(t, &mut acc);
            }
            acc
        };
        // Emit the sameAs relation (all ordered pairs within each non-singleton
        // class, INCLUDING the reflexive pairs — eq-ref restricted to the terms
        // equality actually touches, which SPARQL-entailment answers need).
        for mem in classes.values() {
            for &a in mem {
                for &b in mem {
                    expanded.push([a, o.same_as, b]);
                }
            }
        }
        expanded
    };

    if prof {
        eprintln!("OWL-PROF expand={:.3}", __t.elapsed().as_secs_f64());
    }
    let __t = now();

    // Final ordering: original (sorted) then the genuinely-new facts (sorted + deduplicated —
    // set semantics). The filter and both sorts parallelize (the `rdfs::dedup_derived` pattern).
    let original: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    #[cfg(feature = "parallel")]
    let (mut base, derived) = {
        use rayon::prelude::*;
        let mut derived: Vec<[Id; 3]> = if expanded.len() >= PAR_THRESHOLD {
            expanded.par_iter().copied().filter(|t| !original.contains(t)).collect()
        } else {
            expanded.iter().copied().filter(|t| !original.contains(t)).collect()
        };
        derived.par_sort_unstable();
        derived.dedup();
        let mut base: Vec<[Id; 3]> = original.into_iter().collect();
        base.par_sort_unstable();
        (base, derived)
    };
    #[cfg(not(feature = "parallel"))]
    let (mut base, derived) = {
        let mut derived: Vec<[Id; 3]> = expanded
            .iter()
            .copied()
            .filter(|t| !original.contains(t))
            .collect();
        derived.sort_unstable();
        derived.dedup();
        let mut base: Vec<[Id; 3]> = original.into_iter().collect();
        base.sort_unstable();
        (base, derived)
    };
    let added = derived.len();
    triples.clear();
    triples.append(&mut base);
    triples.extend(derived);
    if prof {
        eprintln!("OWL-PROF finalsort={:.3}", __t.elapsed().as_secs_f64());
    }
    // NOT `triples.len() - before`: duplicate input triples dedup away in the
    // rebuild and the subtraction underflows (see rdfs_closure).
    added
}

/// Detect OWL 2 RL inconsistencies (clashes) in a triple set — run it AFTER materialization
/// so entailed types are present. Returns human-readable clash descriptions (empty = no
/// detected inconsistency). Covers cax-dw (disjointWith), cax-adc (AllDisjointClasses),
/// cls-com (complementOf), cls-nothing (owl:Nothing instances), cls-maxc1, eq-diff1/2/3
/// (sameAs ∩ differentFrom / AllDifferent), prp-asyp / prp-irp (asymmetric & irreflexive
/// violations), prp-pdw / prp-adp (disjoint properties), prp-npa1/2 (negative property
/// assertions), and sameAs forced between distinct literal values (dt-diff ⊢ eq-diff1).
pub fn inconsistencies(dict: &Dict, triples: &[[Id; 3]]) -> Vec<String> {
    use oxrdf::{NamedNode, Term as OTerm};
    let look = |iri: String| dict.lookup(&OTerm::NamedNode(NamedNode::new_unchecked(iri)));
    let ty = look(oxrdf::vocab::rdf::TYPE.as_str().to_string());
    let disjoint_with = look(format!("{OWL}disjointWith"));
    let complement_of = look(format!("{OWL}complementOf"));
    let same_as = look(format!("{OWL}sameAs"));
    let different_from = look(format!("{OWL}differentFrom"));
    let nothing = look(format!("{OWL}Nothing"));
    let thing = look(format!("{OWL}Thing"));
    let on_property = look(format!("{OWL}onProperty"));
    let max_cardinality = look(format!("{OWL}maxCardinality"));
    let max_qual_cardinality = look(format!("{OWL}maxQualifiedCardinality"));
    let on_class_p = look(format!("{OWL}onClass"));
    let all: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
    let mut subj_types: FxHashMap<Id, FxHashSet<Id>> = FxHashMap::default();
    let mut same: FxHashSet<(Id, Id)> = FxHashSet::default();
    let (mut disjoint, mut complement, mut different) = (Vec::new(), Vec::new(), Vec::new());
    let mut on_prop: FxHashMap<Id, Id> = FxHashMap::default();
    let mut max_card0: FxHashSet<Id> = FxHashSet::default();
    let mut max_qcard0: FxHashSet<Id> = FxHashSet::default();
    let mut on_class: FxHashMap<Id, Id> = FxHashMap::default();
    let mut prop_subj: FxHashSet<(Id, Id)> = FxHashSet::default(); // (property, subject-with-a-value)
    for &[s, p, ob] in &all {
        if p == ty {
            subj_types.entry(s).or_default().insert(ob);
        } else if p == disjoint_with {
            disjoint.push((s, ob));
        } else if p == complement_of {
            complement.push((s, ob));
        } else if p == same_as {
            same.insert((s.min(ob), s.max(ob)));
        } else if p == different_from {
            different.push((s, ob));
        } else if p == on_property {
            on_prop.insert(s, ob);
        } else if p == max_cardinality && lit_int(dict, ob) == Some(0) {
            max_card0.insert(s);
        } else if p == max_qual_cardinality && lit_int(dict, ob) == Some(0) {
            max_qcard0.insert(s);
        } else if p == on_class_p {
            on_class.insert(s, ob);
        }
        prop_subj.insert((p, s));
    }
    let mut out = Vec::new();
    let term = |id: Id| dict.term(id).to_string();

    // Extended clash rules need (predicate → pairs), (subject,predicate) → object,
    // and RDF-list access over the same triple set.
    let asym = look(format!("{OWL}AsymmetricProperty"));
    let irrefl = look(format!("{OWL}IrreflexiveProperty"));
    let pdw = look(format!("{OWL}propertyDisjointWith"));
    let adp = look(format!("{OWL}AllDisjointProperties"));
    let adc = look(format!("{OWL}AllDisjointClasses"));
    let all_different = look(format!("{OWL}AllDifferent"));
    let members_p = look(format!("{OWL}members"));
    let distinct_members = look(format!("{OWL}distinctMembers"));
    let source_individual = look(format!("{OWL}sourceIndividual"));
    let assertion_property = look(format!("{OWL}assertionProperty"));
    let target_individual = look(format!("{OWL}targetIndividual"));
    let target_value = look(format!("{OWL}targetValue"));
    let rdf_first = look(format!("{RDF}first"));
    let rdf_rest = look(format!("{RDF}rest"));
    let rdf_nil = look(format!("{RDF}nil"));
    let mut po: FxHashMap<Id, Vec<(Id, Id)>> = FxHashMap::default();
    let mut first_obj: FxHashMap<(Id, Id), Id> = FxHashMap::default();
    for &[s, p, ob] in &all {
        po.entry(p).or_default().push((s, ob));
        first_obj.entry((s, p)).or_insert(ob);
    }
    let list = |head: Id| -> Vec<Id> {
        let mut items = Vec::new();
        let mut cur = head;
        let mut guard = 0;
        while cur != rdf_nil && guard < 10_000 {
            guard += 1;
            match first_obj.get(&(cur, rdf_first)) {
                Some(&f) => items.push(f),
                None => break,
            }
            match first_obj.get(&(cur, rdf_rest)) {
                Some(&r) => cur = r,
                None => break,
            }
        }
        items
    };

    // prp-asyp: an asymmetric property holding in both directions.
    if asym != 0 {
        for (p, ts) in &subj_types {
            if ts.contains(&asym) {
                if let Some(pairs) = po.get(p) {
                    for &(x, y) in pairs {
                        if all.contains(&[y, *p, x]) {
                            out.push(format!(
                                "asymmetric {} holds both ways between {} and {}",
                                term(*p), term(x), term(y)
                            ));
                        }
                    }
                }
            }
        }
    }
    // prp-irp: an irreflexive property relating a node to itself.
    if irrefl != 0 {
        for (p, ts) in &subj_types {
            if ts.contains(&irrefl) {
                if let Some(pairs) = po.get(p) {
                    for &(x, y) in pairs {
                        if x == y {
                            out.push(format!(
                                "irreflexive {} relates {} to itself",
                                term(*p), term(x)
                            ));
                        }
                    }
                }
            }
        }
    }
    // prp-pdw + prp-adp: disjoint properties sharing a (subject, object) pair.
    let mut disjoint_props: Vec<(Id, Id)> = po.get(&pdw).cloned().unwrap_or_default();
    if adp != 0 && members_p != 0 {
        for (z, ts) in &subj_types {
            if ts.contains(&adp) {
                if let Some(&head) = first_obj.get(&(*z, members_p)) {
                    let ms = list(head);
                    for (i, &a) in ms.iter().enumerate() {
                        for &b in &ms[i + 1..] {
                            disjoint_props.push((a, b));
                        }
                    }
                }
            }
        }
    }
    for (p, q) in disjoint_props {
        if let Some(pairs) = po.get(&p) {
            for &(x, y) in pairs {
                if all.contains(&[x, q, y]) {
                    out.push(format!(
                        "disjoint properties {} and {} share the pair ({}, {})",
                        term(p), term(q), term(x), term(y)
                    ));
                }
            }
        }
    }
    // cax-adc: an individual typed by two members of an AllDisjointClasses set.
    if adc != 0 && members_p != 0 {
        for (z, ts) in &subj_types {
            if ts.contains(&adc) {
                if let Some(&head) = first_obj.get(&(*z, members_p)) {
                    let cs = list(head);
                    for (x, xts) in &subj_types {
                        if cs.iter().filter(|c| xts.contains(c)).count() >= 2 {
                            out.push(format!(
                                "{} is typed by two members of an AllDisjointClasses set",
                                term(*x)
                            ));
                        }
                    }
                }
            }
        }
    }
    // prp-npa1/2: a negative property assertion that nevertheless holds.
    if let Some(srcs) = po.get(&source_individual) {
        for &(z, x) in srcs {
            if let Some(&p) = first_obj.get(&(z, assertion_property)) {
                let targets = [
                    first_obj.get(&(z, target_individual)).copied(),
                    first_obj.get(&(z, target_value)).copied(),
                ];
                for y in targets.into_iter().flatten() {
                    if all.contains(&[x, p, y]) {
                        out.push(format!(
                            "negative property assertion violated: {} {} {}",
                            term(x), term(p), term(y)
                        ));
                    }
                }
            }
        }
    }
    // eq-diff2/3: sameAs (or identical) members of an AllDifferent set.
    if all_different != 0 {
        for (z, ts) in &subj_types {
            if ts.contains(&all_different) {
                for mp in [members_p, distinct_members] {
                    if mp == 0 {
                        continue;
                    }
                    if let Some(&head) = first_obj.get(&(*z, mp)) {
                        let xs = list(head);
                        for (i, &a) in xs.iter().enumerate() {
                            for &b in &xs[i + 1..] {
                                if a == b || same.contains(&(a.min(b), a.max(b))) {
                                    out.push(format!(
                                        "AllDifferent members {} and {} are the same",
                                        term(a), term(b)
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // dt-diff ⊢ eq-diff1: equality forced between literals with distinct values
    // (e.g. a FunctionalProperty with two different literal values).
    for &(a, b) in &same {
        if let (oxrdf::Term::Literal(la), oxrdf::Term::Literal(lb)) = (dict.term(a), dict.term(b)) {
            if literal_values_differ(&la, &lb) {
                out.push(format!("distinct literal values {la} and {lb} forced sameAs"));
            }
        }
    }
    // cax-dw / cls-com: an individual typed by two disjoint/complementary classes.
    for (c1, c2) in disjoint.iter().chain(complement.iter()) {
        for (x, ts) in &subj_types {
            if ts.contains(c1) && ts.contains(c2) {
                out.push(format!(
                    "{} is both {} and disjoint/complement {}",
                    term(*x),
                    term(*c1),
                    term(*c2)
                ));
            }
        }
    }
    // eq-diff: x sameAs y but also differentFrom y.
    for (x, y) in &different {
        if same.contains(&((*x).min(*y), (*x).max(*y))) {
            out.push(format!(
                "{} is both sameAs and differentFrom {}",
                term(*x),
                term(*y)
            ));
        }
    }
    // cls-nothing: anything typed owl:Nothing.
    if nothing != 0 {
        for (x, ts) in &subj_types {
            if ts.contains(&nothing) {
                out.push(format!("{} is typed owl:Nothing", term(*x)));
            }
        }
    }
    // cls-maxc1: x typed a maxCardinality-0 restriction on p, yet x has a p-value.
    for &r in &max_card0 {
        if let Some(&p) = on_prop.get(&r) {
            for (x, ts) in &subj_types {
                if ts.contains(&r) && prop_subj.contains(&(p, *x)) {
                    out.push(format!(
                        "{} has a {} value but is maxCardinality 0 on it",
                        term(*x),
                        term(p)
                    ));
                }
            }
        }
    }
    // cls-maxqc1/2 (Profiles §4.3 Table 6): u typed a maxQualifiedCardinality-0
    // restriction [onProperty p; onClass c], yet u has a p-value y with y a c
    // (cls-maxqc1) — any p-value at all when c = owl:Thing (cls-maxqc2). Guarded:
    // only the p-edges of the (few) qualified-cardinality-0 restrictions are scanned.
    for &r in &max_qcard0 {
        if let (Some(&p), Some(&c)) = (on_prop.get(&r), on_class.get(&r)) {
            if let Some(pairs) = po.get(&p) {
                for &(u, y) in pairs {
                    if subj_types.get(&u).is_some_and(|ts| ts.contains(&r))
                        && (c == thing
                            || subj_types.get(&y).is_some_and(|ts| ts.contains(&c)))
                    {
                        out.push(format!(
                            "{} has a {} value{} but is maxQualifiedCardinality 0 on it",
                            term(u),
                            term(p),
                            if c == thing {
                                String::new()
                            } else {
                                format!(" typed {}", term(c))
                            }
                        ));
                    }
                }
            }
        }
    }
    out
}

/// OWL axiom maps (the TBox), rebuilt each fixpoint round.
#[derive(Default)]
struct Axioms {
    inverse: FxHashMap<Id, Vec<Id>>, // p -> inverse properties (both directions)
    equiv_prop: FxHashMap<Id, Vec<Id>>, // p -> equivalent properties (both directions)
    equiv_class: FxHashMap<Id, Vec<Id>>, // c -> equivalent classes (both directions)
    symmetric: FxHashSet<Id>,
    transitive: FxHashSet<Id>,
    functional: FxHashSet<Id>,
    inv_functional: FxHashSet<Id>,
}

impl Axioms {
    fn build(all: &FxHashSet<[Id; 3]>, v: &Vocab, o: &Owl) -> Axioms {
        let mut ax = Axioms::default();
        let bi = |m: &mut FxHashMap<Id, Vec<Id>>, a: Id, b: Id| {
            m.entry(a).or_default().push(b);
            m.entry(b).or_default().push(a);
        };
        for &[s, p, obj] in all {
            if p == o.inverse_of {
                bi(&mut ax.inverse, s, obj);
            } else if p == o.equiv_prop {
                bi(&mut ax.equiv_prop, s, obj);
            } else if p == o.equiv_class {
                bi(&mut ax.equiv_class, s, obj);
            } else if p == v.ty {
                if obj == o.symmetric {
                    ax.symmetric.insert(s);
                } else if obj == o.transitive {
                    ax.transitive.insert(s);
                } else if obj == o.functional {
                    ax.functional.insert(s);
                } else if obj == o.inv_functional {
                    ax.inv_functional.insert(s);
                }
            }
        }
        ax
    }
}

/// Do two literals PROVABLY denote different values? Conservative: only judges
/// the numeric tower (by value), and same-datatype string-family literals (by
/// lexical form); everything else is "cannot tell" = false.
fn literal_values_differ(a: &oxrdf::Literal, b: &oxrdf::Literal) -> bool {
    if a == b {
        return false;
    }
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let numeric = |d: &str| {
        matches!(
            d.strip_prefix(XSD),
            Some(
                "integer" | "decimal" | "long" | "int" | "short" | "byte"
                    | "nonNegativeInteger" | "positiveInteger" | "nonPositiveInteger"
                    | "negativeInteger" | "unsignedLong" | "unsignedInt" | "unsignedShort"
                    | "unsignedByte"
            )
        )
    };
    let (da, db) = (a.datatype(), b.datatype());
    if numeric(da.as_str()) && numeric(db.as_str()) {
        if let (Ok(x), Ok(y)) = (a.value().parse::<f64>(), b.value().parse::<f64>()) {
            return x != y;
        }
        return false;
    }
    if da == db && a.language() == b.language() {
        return matches!(da.as_str().strip_prefix(XSD), Some("string"))
            || da.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::vocab::rdf;
    use oxrdf::{NamedNode, Term};

    fn ex(dict: &mut Dict, local: &str) -> Id {
        dict.intern_iri(&format!("http://ex/{local}"))
    }
    fn owl(dict: &mut Dict, frag: &str) -> Id {
        dict.intern_iri(&format!("{OWL}{frag}"))
    }
    fn has(dict: &Dict, set: &FxHashSet<[Id; 3]>, s: &str, p: &str, o: &str) -> bool {
        let g =
            |iri: &str| dict.lookup(&Term::NamedNode(NamedNode::new_unchecked(iri.to_string())));
        let (si, pi, oi) = (g(s), g(p), g(o));
        si != 0 && pi != 0 && oi != 0 && set.contains(&[si, pi, oi])
    }

    #[test]
    fn inverse_symmetric_transitive() {
        let mut dict = Dict::new();
        let (parent, child, a, b, c) = (
            ex(&mut dict, "parentOf"),
            ex(&mut dict, "childOf"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
            ex(&mut dict, "c"),
        );
        let (anc, knows) = (ex(&mut dict, "ancestorOf"), ex(&mut dict, "knows"));
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let inv = owl(&mut dict, "inverseOf");
        let trans = owl(&mut dict, "TransitiveProperty");
        let sym = owl(&mut dict, "SymmetricProperty");
        let mut triples = vec![
            [parent, inv, child], // parentOf inverseOf childOf
            [anc, ty, trans],     // ancestorOf a TransitiveProperty
            [knows, ty, sym],     // knows a SymmetricProperty
            [a, parent, b],       // a parentOf b
            [a, anc, b],
            [b, anc, c],   // a anc b ; b anc c
            [a, knows, b], // a knows b
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            has(
                &dict,
                &set,
                "http://ex/b",
                "http://ex/childOf",
                "http://ex/a"
            ),
            "prp-inv"
        );
        assert!(
            has(
                &dict,
                &set,
                "http://ex/a",
                "http://ex/ancestorOf",
                "http://ex/c"
            ),
            "prp-trp"
        );
        assert!(
            has(&dict, &set, "http://ex/b", "http://ex/knows", "http://ex/a"),
            "prp-symp"
        );
    }

    #[test]
    fn sameas_and_functional() {
        // ex:hasSSN a FunctionalProperty ; a hasSSN s ; a hasSSN s2  ⊢  s sameAs s2.
        // Then eq-rep: (s p o) carries to s2.
        let mut dict = Dict::new();
        let (ssn, a, s1, s2, mark) = (
            ex(&mut dict, "hasSSN"),
            ex(&mut dict, "a"),
            ex(&mut dict, "s1"),
            ex(&mut dict, "s2"),
            ex(&mut dict, "marker"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let func = owl(&mut dict, "FunctionalProperty");
        let p = ex(&mut dict, "p");
        let mut triples = vec![
            [ssn, ty, func],
            [a, ssn, s1],
            [a, ssn, s2],  // ⊢ s1 sameAs s2
            [s1, p, mark], // eq-rep ⊢ s2 p marker
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            has(
                &dict,
                &set,
                "http://ex/s1",
                &format!("{OWL}sameAs"),
                "http://ex/s2"
            ),
            "prp-fp ⊢ sameAs"
        );
        assert!(
            has(
                &dict,
                &set,
                "http://ex/s2",
                "http://ex/p",
                "http://ex/marker"
            ),
            "eq-rep substitution"
        );
    }

    // [SONNET-4.6] sq-y4ll5: direct production-path coverage for prp-ifp and the
    // OWL-RL guard against the excluded ReflexiveObjectProperty feature.
    #[test]
    fn sameas_and_inverse_functional() {
        // ex:hasSSN a InverseFunctionalProperty ; a hasSSN s ; b hasSSN s
        // ⊢ a sameAs b. Then eq-rep carries a's marker edge to b.
        let mut dict = Dict::new();
        let (ssn, a, b, value, mark) = (
            ex(&mut dict, "hasSSN"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
            ex(&mut dict, "value"),
            ex(&mut dict, "marker"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let inv_func = owl(&mut dict, "InverseFunctionalProperty");
        let p = ex(&mut dict, "p");
        let mut triples =
            vec![[ssn, ty, inv_func], [a, ssn, value], [b, ssn, value], [a, p, mark]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        let same_as = [OWL, "sameAs"].concat();
        assert!(
            has(
                &dict,
                &set,
                "http://ex/a",
                &same_as,
                "http://ex/b"
            ),
            "prp-ifp ⊢ sameAs"
        );
        assert!(
            has(&dict, &set, "http://ex/b", "http://ex/p", "http://ex/marker"),
            "eq-rep substitution after prp-ifp"
        );
    }

    #[test]
    fn reflexive_property_does_not_materialize_self_edges() {
        // ReflexiveObjectProperty is excluded from OWL 2 RL. Merely declaring :p
        // owl:ReflexiveProperty must therefore not invent :x :p :x for graph terms.
        let mut dict = Dict::new();
        let (p, x, y) = (ex(&mut dict, "p"), ex(&mut dict, "x"), ex(&mut dict, "y"));
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let reflexive = owl(&mut dict, "ReflexiveProperty");
        let mut triples = vec![[p, ty, reflexive], [x, p, y]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            !set.contains(&[x, p, x]),
            "OWL-RL must not derive a reflexive edge for :x"
        );
        assert!(
            !set.contains(&[y, p, y]),
            "OWL-RL must not derive a reflexive edge for :y"
        );
    }

    fn rdf(dict: &mut Dict, frag: &str) -> Id {
        dict.intern_iri(&format!(
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#{frag}"
        ))
    }
    /// Build an RDF list of `items` into `triples`, returning the head node id.
    fn list(dict: &mut Dict, triples: &mut Vec<[Id; 3]>, items: &[Id]) -> Id {
        let (first, rest, nil) = (rdf(dict, "first"), rdf(dict, "rest"), rdf(dict, "nil"));
        let mut head = nil;
        for (k, &it) in items.iter().enumerate().rev() {
            let node = dict.intern_blank(&format!("_l{k}_{it}"));
            triples.push([node, first, it]);
            triples.push([node, rest, head]);
            head = node;
        }
        head
    }

    #[test]
    fn property_chain() {
        // :uncleOf owl:propertyChainAxiom ( :parentOf :brotherOf ) ; a parentOf b ; b brotherOf c
        // ⊢ a uncleOf c.
        let mut dict = Dict::new();
        let (uncle, parent, brother, a, b, c) = (
            ex(&mut dict, "uncleOf"),
            ex(&mut dict, "parentOf"),
            ex(&mut dict, "brotherOf"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
            ex(&mut dict, "c"),
        );
        let pca = owl(&mut dict, "propertyChainAxiom");
        let mut triples = vec![[a, parent, b], [b, brother, c]];
        let chain = list(&mut dict, &mut triples, &[parent, brother]);
        triples.push([uncle, pca, chain]);
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            has(
                &dict,
                &set,
                "http://ex/a",
                "http://ex/uncleOf",
                "http://ex/c"
            ),
            "prp-spo2 property chain"
        );
    }

    #[test]
    fn owl_no_features_routes_to_single_pass_with_scm_dom_rng() {
        // No OWL-specific feature → the OWL-RL closure is RDFS + scm-dom/rng, computed by the
        // single-pass fast path. Validates the routing produces rdfs7/2/3/9 + scm-dom1/2/rng1.
        let mut dict = Dict::new();
        let r = |d: &mut Dict, f: &str| d.intern_iri(&format!("http://www.w3.org/2000/01/rdf-schema#{f}"));
        let (sc, sp, dom, rng) =
            (r(&mut dict, "subClassOf"), r(&mut dict, "subPropertyOf"), r(&mut dict, "domain"), r(&mut dict, "range"));
        let ty = rdf(&mut dict, "type");
        let (c0, c1, d0, d1, p, q, x, y) = (
            ex(&mut dict, "c0"),
            ex(&mut dict, "c1"),
            ex(&mut dict, "d0"),
            ex(&mut dict, "d1"),
            ex(&mut dict, "p"),
            ex(&mut dict, "q"),
            ex(&mut dict, "x"),
            ex(&mut dict, "y"),
        );
        let mut triples =
            vec![[c0, sc, c1], [d0, sc, d1], [p, dom, c0], [p, rng, d0], [q, sp, p], [x, q, y]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[x, p, y]), "rdfs7 (q subPropertyOf p)");
        assert!(set.contains(&[x, ty, c0]) && set.contains(&[x, ty, c1]), "rdfs2 domain + rdfs9 up subclass");
        assert!(set.contains(&[y, ty, d0]) && set.contains(&[y, ty, d1]), "rdfs3 range + rdfs9 up subclass");
        assert!(set.contains(&[p, dom, c1]), "scm-dom1 (domain up subclass)");
        assert!(set.contains(&[q, dom, c0]), "scm-dom2 (domain down subproperty)");
        assert!(set.contains(&[p, rng, d1]), "scm-rng1 (range up subclass)");
    }

    #[test]
    fn restriction_has_value_and_some_values() {
        // hasValue: R1 = [onProperty :status; hasValue :active]; x a R1 ⊢ x :status :active.
        // someValuesFrom: R2 = [onProperty :hasPet; someValuesFrom :Dog]; x hasPet d, d a Dog ⊢ x a R2.
        let mut dict = Dict::new();
        let (status, active, haspet, dog, x, d) = (
            ex(&mut dict, "status"),
            ex(&mut dict, "active"),
            ex(&mut dict, "hasPet"),
            ex(&mut dict, "Dog"),
            ex(&mut dict, "x"),
            ex(&mut dict, "d"),
        );
        let (r1, r2) = (dict.intern_blank("_R1"), dict.intern_blank("_R2"));
        let ty = rdf(&mut dict, "type");
        let (onp, hv, svf) = (
            owl(&mut dict, "onProperty"),
            owl(&mut dict, "hasValue"),
            owl(&mut dict, "someValuesFrom"),
        );
        let mut triples = vec![
            [r1, onp, status],
            [r1, hv, active],
            [x, ty, r1],
            [r2, onp, haspet],
            [r2, svf, dog],
            [x, haspet, d],
            [d, ty, dog],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            has(
                &dict,
                &set,
                "http://ex/x",
                "http://ex/status",
                "http://ex/active"
            ),
            "cls-hv1"
        );
        // x is in R2 because it has a Dog pet:
        assert!(set.contains(&[x, ty, r2]), "cls-svf1");
    }

    #[test]
    fn has_key_identifies() {
        // :Person owl:hasKey ( :ssn ) ; a,b both :Person with the same :ssn ⊢ a sameAs b.
        let mut dict = Dict::new();
        let (person, ssn, a, b) = (
            ex(&mut dict, "Person"),
            ex(&mut dict, "ssn"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let ty = rdf(&mut dict, "type");
        let hk = owl(&mut dict, "hasKey");
        let v = dict.intern_lit("123", "http://www.w3.org/2001/XMLSchema#string", None);
        let mut triples = vec![[a, ty, person], [a, ssn, v], [b, ty, person], [b, ssn, v]];
        let keylist = list(&mut dict, &mut triples, &[ssn]);
        triples.push([person, hk, keylist]);
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        let sa = owl(&mut dict, "sameAs");
        assert!(
            set.contains(&[a, sa, b]) || set.contains(&[b, sa, a]),
            "prp-key ⊢ sameAs"
        );
    }

    #[test]
    fn max_cardinality_one_and_zero() {
        // maxCardinality 1: the two spouses of :a are sameAs.
        let mut dict = Dict::new();
        let (spouse, a, x, y) = (
            ex(&mut dict, "spouse"),
            ex(&mut dict, "a"),
            ex(&mut dict, "x"),
            ex(&mut dict, "y"),
        );
        let r = dict.intern_blank("_R1");
        let ty = rdf(&mut dict, "type");
        let (onp, mc) = (
            owl(&mut dict, "onProperty"),
            owl(&mut dict, "maxCardinality"),
        );
        let one = dict.intern_lit("1", "http://www.w3.org/2001/XMLSchema#integer", None);
        let mut triples = vec![
            [r, onp, spouse],
            [r, mc, one],
            [a, ty, r],
            [a, spouse, x],
            [a, spouse, y],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        let sa = owl(&mut dict, "sameAs");
        assert!(
            set.contains(&[x, sa, y]) || set.contains(&[y, sa, x]),
            "cls-maxc2 ⊢ sameAs"
        );

        // maxCardinality 0: :b typed a no-child restriction yet has a child ⊢ inconsistent.
        let mut d2 = Dict::new();
        let (child, b) = (ex(&mut d2, "child"), ex(&mut d2, "b"));
        let r2 = d2.intern_blank("_R0");
        let ty = rdf(&mut d2, "type");
        let (onp, mc) = (owl(&mut d2, "onProperty"), owl(&mut d2, "maxCardinality"));
        let zero = d2.intern_lit("0", "http://www.w3.org/2001/XMLSchema#integer", None);
        let mut t2 = vec![
            [r2, onp, child],
            [r2, mc, zero],
            [b, ty, r2],
            [b, child, ex(&mut d2, "kid")],
        ];
        materialize_owl_rl(&mut d2, &mut t2);
        assert!(
            !inconsistencies(&d2, &t2).is_empty(),
            "cls-maxc1 clash detected"
        );
    }

    #[test]
    fn max_qualified_cardinality() {
        // R = [onProperty :parent; maxQualifiedCardinality 1; onClass :Mother];
        // :x a R, :x :parent :a, :x :parent :b, :a a :Mother, :b a :Mother ⊢ :a sameAs :b.
        let mut dict = Dict::new();
        let (parent, mother, x, a, b) = (
            ex(&mut dict, "parent"),
            ex(&mut dict, "Mother"),
            ex(&mut dict, "x"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let r = dict.intern_blank("_RQ");
        let ty = rdf(&mut dict, "type");
        let (onp, mqc, onc) = (
            owl(&mut dict, "onProperty"),
            owl(&mut dict, "maxQualifiedCardinality"),
            owl(&mut dict, "onClass"),
        );
        let one = dict.intern_lit("1", "http://www.w3.org/2001/XMLSchema#integer", None);
        let mut triples = vec![
            [r, onp, parent],
            [r, mqc, one],
            [r, onc, mother],
            [x, ty, r],
            [x, parent, a],
            [x, parent, b],
            [a, ty, mother],
            [b, ty, mother],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        let sa = owl(&mut dict, "sameAs");
        assert!(
            set.contains(&[a, sa, b]) || set.contains(&[b, sa, a]),
            "cls-maxqc ⊢ sameAs"
        );
    }

    #[test]
    fn one_of_members_typed() {
        // cls-oo: :Weekday owl:oneOf ( :mon :tue ) ⊢ :mon a :Weekday ; :tue a :Weekday.
        // Hand-computed closure: the 5 input triples (oneOf head + 2 list cells) plus
        // EXACTLY the two cls-oo type assertions — no other rule fires.
        let mut dict = Dict::new();
        let (weekday, mon, tue) = (
            ex(&mut dict, "Weekday"),
            ex(&mut dict, "mon"),
            ex(&mut dict, "tue"),
        );
        let (ty, oo) = (rdf(&mut dict, "type"), owl(&mut dict, "oneOf"));
        let mut triples = Vec::new();
        let l = list(&mut dict, &mut triples, &[mon, tue]);
        triples.push([weekday, oo, l]);
        let input: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        let added = materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[mon, ty, weekday]), "cls-oo first member");
        assert!(set.contains(&[tue, ty, weekday]), "cls-oo second member");
        assert_eq!(added, 2, "exactly the two cls-oo conclusions");
        assert_eq!(set.len(), input.len() + 2);
    }

    #[test]
    fn max_qualified_cardinality_zero_clashes() {
        // cls-maxqc1: R = [onProperty :p; maxQualifiedCardinality 0; onClass :C];
        // :u a R ; :u :p :y ; :y a :C  → inconsistent.
        let mut dict = Dict::new();
        let (p, c, u, y) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "C"),
            ex(&mut dict, "u"),
            ex(&mut dict, "y"),
        );
        let r = dict.intern_blank("_RQ0");
        let ty = rdf(&mut dict, "type");
        let (onp, mqc, onc) = (
            owl(&mut dict, "onProperty"),
            owl(&mut dict, "maxQualifiedCardinality"),
            owl(&mut dict, "onClass"),
        );
        let zero = dict.intern_lit("0", "http://www.w3.org/2001/XMLSchema#nonNegativeInteger", None);
        let mut triples = vec![
            [r, onp, p],
            [r, mqc, zero],
            [r, onc, c],
            [u, ty, r],
            [u, p, y],
            [y, ty, c],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        assert!(!inconsistencies(&dict, &triples).is_empty(), "cls-maxqc1 clash");

        // Consistent variant: the p-value is NOT a :C — no clash (the qualification matters).
        let mut d2 = Dict::new();
        let (p, c, u, y) = (ex(&mut d2, "p"), ex(&mut d2, "C"), ex(&mut d2, "u"), ex(&mut d2, "y"));
        let r = d2.intern_blank("_RQ0");
        let ty = rdf(&mut d2, "type");
        let (onp, mqc, onc) = (
            owl(&mut d2, "onProperty"),
            owl(&mut d2, "maxQualifiedCardinality"),
            owl(&mut d2, "onClass"),
        );
        let zero = d2.intern_lit("0", "http://www.w3.org/2001/XMLSchema#nonNegativeInteger", None);
        let mut t2 = vec![[r, onp, p], [r, mqc, zero], [r, onc, c], [u, ty, r], [u, p, y]];
        materialize_owl_rl(&mut d2, &mut t2);
        assert!(inconsistencies(&d2, &t2).is_empty(), "untyped filler: no cls-maxqc1 clash");

        // cls-maxqc2: onClass owl:Thing — ANY p-value clashes.
        let mut d3 = Dict::new();
        let (p, u, y) = (ex(&mut d3, "p"), ex(&mut d3, "u"), ex(&mut d3, "y"));
        let r = d3.intern_blank("_RQ0");
        let ty = rdf(&mut d3, "type");
        let (onp, mqc, onc, thing) = (
            owl(&mut d3, "onProperty"),
            owl(&mut d3, "maxQualifiedCardinality"),
            owl(&mut d3, "onClass"),
            owl(&mut d3, "Thing"),
        );
        let zero = d3.intern_lit("0", "http://www.w3.org/2001/XMLSchema#nonNegativeInteger", None);
        let mut t3 = vec![[r, onp, p], [r, mqc, zero], [r, onc, thing], [u, ty, r], [u, p, y]];
        materialize_owl_rl(&mut d3, &mut t3);
        assert!(!inconsistencies(&d3, &t3).is_empty(), "cls-maxqc2 clash");
    }

    #[test]
    fn scm_svf1_restriction_subsumption() {
        // scm-svf1: R1 = [onProperty :hasPet; someValuesFrom :Dog],
        // R2 = [onProperty :hasPet; someValuesFrom :Animal], :Dog ⊑ :Animal ⊢ R1 ⊑ R2.
        // Hand-computed closure: input + EXACTLY {R1 ⊑ R2 (scm-svf1), x a R2 (rdfs9)}.
        let mut dict = Dict::new();
        let (haspet, dog, animal, x) = (
            ex(&mut dict, "hasPet"),
            ex(&mut dict, "Dog"),
            ex(&mut dict, "Animal"),
            ex(&mut dict, "x"),
        );
        let (r1, r2) = (dict.intern_blank("_R1"), dict.intern_blank("_R2"));
        let ty = rdf(&mut dict, "type");
        let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let (onp, svf) = (owl(&mut dict, "onProperty"), owl(&mut dict, "someValuesFrom"));
        let mut triples = vec![
            [dog, sc, animal],
            [r1, onp, haspet],
            [r1, svf, dog],
            [r2, onp, haspet],
            [r2, svf, animal],
            [x, ty, r1],
        ];
        let added = materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[r1, sc, r2]), "scm-svf1");
        assert!(set.contains(&[x, ty, r2]), "rdfs9 over the scm-svf1 edge (fixpoint feed)");
        assert_eq!(added, 2, "exactly scm-svf1 + rdfs9");
    }

    #[test]
    fn scm_svf2_and_hv_restriction_subsumption() {
        // scm-svf2: same filler :C, :p1 ⊑p :p2 ⊢ R1 ⊑ R2.
        // scm-hv:   same value :i, :q1 ⊑p :q2 ⊢ H1 ⊑ H2.
        let mut dict = Dict::new();
        let (p1, p2, c, q1, q2, i) = (
            ex(&mut dict, "p1"),
            ex(&mut dict, "p2"),
            ex(&mut dict, "C"),
            ex(&mut dict, "q1"),
            ex(&mut dict, "q2"),
            ex(&mut dict, "i"),
        );
        let (r1, r2, h1, h2) = (
            dict.intern_blank("_R1"),
            dict.intern_blank("_R2"),
            dict.intern_blank("_H1"),
            dict.intern_blank("_H2"),
        );
        let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let sp = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subPropertyOf");
        let (onp, svf, hv) = (
            owl(&mut dict, "onProperty"),
            owl(&mut dict, "someValuesFrom"),
            owl(&mut dict, "hasValue"),
        );
        let mut triples = vec![
            [p1, sp, p2],
            [r1, onp, p1],
            [r1, svf, c],
            [r2, onp, p2],
            [r2, svf, c],
            [q1, sp, q2],
            [h1, onp, q1],
            [h1, hv, i],
            [h2, onp, q2],
            [h2, hv, i],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[r1, sc, r2]), "scm-svf2");
        assert!(!set.contains(&[r2, sc, r1]), "scm-svf2 is directional");
        assert!(set.contains(&[h1, sc, h2]), "scm-hv");
        assert!(!set.contains(&[h2, sc, h1]), "scm-hv is directional");
    }

    #[test]
    fn scm_avf_restriction_subsumption() {
        // scm-avf1: same property, :Dog ⊑c :Animal ⊢ R1 ⊑ R2 (covariant in the filler).
        // scm-avf2: same filler, :p1 ⊑p :p2 ⊢ R2 ⊑ R1 — the conclusion REVERSES
        // (allValuesFrom is contravariant in the property).
        let mut dict = Dict::new();
        let (p, dog, animal, p1, p2, c) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "Dog"),
            ex(&mut dict, "Animal"),
            ex(&mut dict, "p1"),
            ex(&mut dict, "p2"),
            ex(&mut dict, "C"),
        );
        let (r1, r2, a1, a2) = (
            dict.intern_blank("_R1"),
            dict.intern_blank("_R2"),
            dict.intern_blank("_A1"),
            dict.intern_blank("_A2"),
        );
        let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let sp = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subPropertyOf");
        let (onp, avf) = (owl(&mut dict, "onProperty"), owl(&mut dict, "allValuesFrom"));
        let mut triples = vec![
            // scm-avf1 instance
            [dog, sc, animal],
            [r1, onp, p],
            [r1, avf, dog],
            [r2, onp, p],
            [r2, avf, animal],
            // scm-avf2 instance
            [p1, sp, p2],
            [a1, onp, p1],
            [a1, avf, c],
            [a2, onp, p2],
            [a2, avf, c],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[r1, sc, r2]), "scm-avf1");
        assert!(!set.contains(&[r2, sc, r1]), "scm-avf1 is directional");
        assert!(set.contains(&[a2, sc, a1]), "scm-avf2 (reversed conclusion)");
        assert!(!set.contains(&[a1, sc, a2]), "scm-avf2 derives ONLY the reversed edge");
    }

    #[test]
    fn thing_nothing_typed_class_when_occurring() {
        // cls-thing: a graph that mentions owl:Thing gets `owl:Thing a owl:Class`
        // (premise-free axiom, occurrence-guarded); ditto cls-nothing1 for owl:Nothing.
        let mut dict = Dict::new();
        let x = ex(&mut dict, "x");
        let ty = rdf(&mut dict, "type");
        let (thing, class) = (owl(&mut dict, "Thing"), owl(&mut dict, "Class"));
        let mut triples = vec![[x, ty, thing]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[thing, ty, class]), "cls-thing");
        let nothing = owl(&mut dict, "Nothing");
        assert!(
            !set.contains(&[nothing, ty, class]),
            "owl:Nothing does not occur — guard keeps it out"
        );

        // And the Nothing side.
        let mut d2 = Dict::new();
        let (c2, ty2) = (ex(&mut d2, "Empty"), rdf(&mut d2, "type"));
        let (nothing2, class2) = (owl(&mut d2, "Nothing"), owl(&mut d2, "Class"));
        let sc2 = d2.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let mut t2 = vec![[c2, sc2, nothing2]];
        materialize_owl_rl(&mut d2, &mut t2);
        let s2: FxHashSet<[Id; 3]> = t2.iter().copied().collect();
        assert!(s2.contains(&[nothing2, ty2, class2]), "cls-nothing1");
    }

    #[test]
    fn consistency_disjoint_clash() {
        // :Cat owl:disjointWith :Dog ; :felix a :Cat ; :felix a :Dog  → inconsistent.
        let mut dict = Dict::new();
        let (cat, dog, felix) = (
            ex(&mut dict, "Cat"),
            ex(&mut dict, "Dog"),
            ex(&mut dict, "felix"),
        );
        let ty = rdf(&mut dict, "type");
        let dw = owl(&mut dict, "disjointWith");
        let mut triples = vec![[cat, dw, dog], [felix, ty, cat], [felix, ty, dog]];
        materialize_owl_rl(&mut dict, &mut triples);
        let clashes = inconsistencies(&dict, &triples);
        assert!(!clashes.is_empty(), "disjointWith clash should be detected");

        // A consistent variant (felix only a Cat) must report none.
        let mut dict2 = Dict::new();
        let (cat, dog, felix) = (
            ex(&mut dict2, "Cat"),
            ex(&mut dict2, "Dog"),
            ex(&mut dict2, "felix"),
        );
        let ty = rdf(&mut dict2, "type");
        let dw = owl(&mut dict2, "disjointWith");
        let mut t2 = vec![[cat, dw, dog], [felix, ty, cat]];
        materialize_owl_rl(&mut dict2, &mut t2);
        assert!(
            inconsistencies(&dict2, &t2).is_empty(),
            "consistent graph reports no clash"
        );
    }

    #[test]
    fn scm_domain_range_propagation() {
        // :p2 subPropertyOf :p ; :p domain :C ; :C subClassOf :D
        // ⊢ :p domain :D (scm-dom1, up subClassOf) and :p2 domain :C (scm-dom2, down subPropertyOf).
        let mut dict = Dict::new();
        let (p, p2, cc, dd) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "p2"),
            ex(&mut dict, "C"),
            ex(&mut dict, "D"),
        );
        let rdfs = |d: &mut Dict, f: &str| {
            d.intern_iri(&format!("http://www.w3.org/2000/01/rdf-schema#{f}"))
        };
        let (sp, dom, sc) = (
            rdfs(&mut dict, "subPropertyOf"),
            rdfs(&mut dict, "domain"),
            rdfs(&mut dict, "subClassOf"),
        );
        let mut triples = vec![[p2, sp, p], [p, dom, cc], [cc, sc, dd]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            set.contains(&[p, dom, dd]),
            "scm-dom1: domain up subClassOf"
        );
        assert!(
            set.contains(&[p2, dom, cc]),
            "scm-dom2: domain down subPropertyOf"
        );
    }

    #[test]
    fn scm_schema_subclass() {
        // equivalentClass ⊢ subClassOf both ways; intersectionOf ⊢ c subClassOf each member.
        let mut dict = Dict::new();
        let (human, person, mother, woman, parent) = (
            ex(&mut dict, "Human"),
            ex(&mut dict, "Person"),
            ex(&mut dict, "Mother"),
            ex(&mut dict, "Woman"),
            ex(&mut dict, "Parent"),
        );
        let (eqc, io) = (
            owl(&mut dict, "equivalentClass"),
            owl(&mut dict, "intersectionOf"),
        );
        let sco = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let mut triples = vec![[human, eqc, person]];
        let l = list(&mut dict, &mut triples, &[woman, parent]);
        triples.push([mother, io, l]);
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[human, sco, person]), "scm-eqc forward");
        assert!(set.contains(&[person, sco, human]), "scm-eqc backward");
        assert!(
            set.contains(&[mother, sco, woman]) && set.contains(&[mother, sco, parent]),
            "scm-int"
        );
    }

    #[test]
    fn intersection_of() {
        // :Mother owl:intersectionOf ( :Woman :Parent ) ; x a Woman ; x a Parent ⊢ x a Mother.
        let mut dict = Dict::new();
        let (mother, woman, parent, x) = (
            ex(&mut dict, "Mother"),
            ex(&mut dict, "Woman"),
            ex(&mut dict, "Parent"),
            ex(&mut dict, "x"),
        );
        let (ty, io) = (rdf(&mut dict, "type"), owl(&mut dict, "intersectionOf"));
        let mut triples = vec![[x, ty, woman], [x, ty, parent]];
        let l = list(&mut dict, &mut triples, &[woman, parent]);
        triples.push([mother, io, l]);
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            set.contains(&[x, ty, mother]),
            "cls-int1 intersection membership"
        );
    }

    #[test]
    fn equivalent_class_and_property() {
        let mut dict = Dict::new();
        let (human, person, x) = (
            ex(&mut dict, "Human"),
            ex(&mut dict, "Person"),
            ex(&mut dict, "x"),
        );
        let (likes, enjoys, y) = (
            ex(&mut dict, "likes"),
            ex(&mut dict, "enjoys"),
            ex(&mut dict, "y"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let eqc = owl(&mut dict, "equivalentClass");
        let eqp = owl(&mut dict, "equivalentProperty");
        let mut triples = vec![
            [human, eqc, person],
            [x, ty, human],
            [likes, eqp, enjoys],
            [x, likes, y],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            has(
                &dict,
                &set,
                "http://ex/x",
                rdf::TYPE.as_str(),
                "http://ex/Person"
            ),
            "cax-eqc"
        );
        assert!(
            has(
                &dict,
                &set,
                "http://ex/x",
                "http://ex/enjoys",
                "http://ex/y"
            ),
            "prp-eqp"
        );
    }

    // [OPUS-4.8] sq-350ms (epic sq-pbz04) — OWL 2 RL COMPLETENESS HARDENING.
    //
    // The single-rule unit tests above each fire ONE rule on an already-asserted
    // premise. The four guards below pin the harder MULTI-ROUND interactions
    // where a real completeness bug would hide: a rule whose premise only becomes
    // true AFTER an earlier round derived it. They are the load-bearing
    // "the closure is COMPLETE for the assertion-style RL/RDF rules" assertions —
    // the conformance OWL-RL row (78 pass + 13 documented divergences, all
    // PROVABLY outside the RL profile — see `DOCUMENTED_DIVERGENCES` + the
    // research/inference-completeness-audit.md §2 table) sits at the genuine RL
    // ceiling, so the suite cannot catch a regression in these compositions; these
    // hand-computed-closure tests do. Each is a TRUE OWL 2 RL/RDF entailment
    // (W3C OWL 2 Profiles §4.3 Tables 5/6/9), derived by the production
    // `materialize_owl_rl` path — no special-casing.

    #[test]
    fn cls_svf1_fires_on_a_subclass_derived_type() {
        // cls-svf1 (Table 6) is COMPLETE only if it fires on a value whose
        // someValuesFrom-class membership was DERIVED, not asserted:
        //   :R owl:someValuesFrom :C ; :R owl:onProperty :p ;
        //   :x :p :u ; :u a :D ; :D rdfs:subClassOf :C
        // round 1 (rdfs9): :u a :C ; round 2 (cls-svf1): :x a :R.
        // A materializer that only indexed ASSERTED types would miss :x a :R.
        let mut dict = Dict::new();
        let (r, p, c, d, x, u) = (
            ex(&mut dict, "R"),
            ex(&mut dict, "p"),
            ex(&mut dict, "C"),
            ex(&mut dict, "D"),
            ex(&mut dict, "x"),
            ex(&mut dict, "u"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let svf = owl(&mut dict, "someValuesFrom");
        let onp = owl(&mut dict, "onProperty");
        let sc = dict.intern_iri(oxrdf::vocab::rdfs::SUB_CLASS_OF.as_str());
        let mut triples = vec![[r, svf, c], [r, onp, p], [x, p, u], [u, ty, d], [d, sc, c]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[x, ty, r]), "cls-svf1 over a DERIVED filler type");
    }

    #[test]
    fn subproperty_of_a_transitive_property_closes_transitively() {
        // prp-spo1 (rdfs7) feeding prp-trp (Table 5): a property declared a
        // subPropertyOf a TRANSITIVE property must have its super-property edges
        // transitively closed.
        //   :p a owl:TransitiveProperty ; :q rdfs:subPropertyOf :p ;
        //   :a :q :b ; :b :q :c   ⊢   :a :p :c
        // (round 1: :a :p :b, :b :p :c via spo1; round 2: :a :p :c via prp-trp).
        let mut dict = Dict::new();
        let (p, q, a, b, c) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "q"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
            ex(&mut dict, "c"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let trans = owl(&mut dict, "TransitiveProperty");
        let sp = dict.intern_iri(oxrdf::vocab::rdfs::SUB_PROPERTY_OF.as_str());
        let mut triples = vec![[p, ty, trans], [q, sp, p], [a, q, b], [b, q, c]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[a, p, c]), "spo1 ⊕ prp-trp (transitive super-property)");
    }

    #[test]
    fn intersection_membership_propagates_through_equivalent_class() {
        // cls-int1 (Table 6) feeding cax-eqc1 (Table 6): the intersection-class
        // membership derived in round 1 must propagate through an equivalentClass
        // axiom in round 2.
        //   :C owl:intersectionOf ( :A :B ) ; :C owl:equivalentClass :E ;
        //   :x a :A ; :x a :B   ⊢   :x a :C (int1)   ⊢   :x a :E (cax-eqc1).
        let mut dict = Dict::new();
        let (cc, a, b, e, x) = (
            ex(&mut dict, "C"),
            ex(&mut dict, "A"),
            ex(&mut dict, "B"),
            ex(&mut dict, "E"),
            ex(&mut dict, "x"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let io = owl(&mut dict, "intersectionOf");
        let eqc = owl(&mut dict, "equivalentClass");
        let mut triples = vec![[x, ty, a], [x, ty, b], [cc, eqc, e]];
        let l = list(&mut dict, &mut triples, &[a, b]);
        triples.push([cc, io, l]);
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[x, ty, cc]), "cls-int1 membership");
        assert!(set.contains(&[x, ty, e]), "cax-eqc1 over the DERIVED int1 membership");
    }

    #[test]
    fn equivalent_property_chain_closes_transitively() {
        // prp-eqp1/2 (Table 5) is a join over the BOTH-DIRECTIONS equivalence
        // relation, so a CHAIN of equivalences must transport an assertion across
        // every link:
        //   :p1 owl:equivalentProperty :p2 ; :p2 owl:equivalentProperty :p3 ;
        //   :a :p1 :b   ⊢   :a :p3 :b
        // (round 1: :a :p2 :b; round 2: :a :p3 :b). An equivalence map that was
        // not transitively re-joined each round would stop at :p2.
        let mut dict = Dict::new();
        let (p1, p2, p3, a, b) = (
            ex(&mut dict, "p1"),
            ex(&mut dict, "p2"),
            ex(&mut dict, "p3"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let eqp = owl(&mut dict, "equivalentProperty");
        let mut triples = vec![[p1, eqp, p2], [p2, eqp, p3], [a, p1, b]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[a, p3, b]), "prp-eqp across a 2-link equivalence chain");
    }

    // [OPUS-4.8] sq-350ms — SOUNDNESS guard (the other half of "completeness
    // hardening": adding coverage must never tip the closure into OVER-inference).
    // The conclusions below are the documented NON-RL divergences whose shape
    // is closest to something a careless extra rule might wrongly derive — a
    // differentFrom of disjoint-property fillers (the prp-pdw contrapositive) and
    // a sameAs that no functional/IFP premise licenses. RL has NO rule producing
    // owl:differentFrom, and prp-fp needs the SAME subject; the materializer must
    // derive NEITHER. (These are exactly the conformance OWL-RL divergences
    // New-Feature-DisjointObjectProperties-001 and owl2-rl-rules-fp-differentFrom,
    // pinned here as in-crate soundness guards so the divergence rationale and the
    // code can never silently disagree.)
    //
    // [SONNET-4.6] sq-qs485 extends the same pattern to two further entries of that
    // divergence list, chosen because a careless extra rule could plausibly
    // over-derive their conclusions: owl2-rl-rules-ifp-differentFrom (the prp-ifp
    // contrapositive, symmetric to the prp-fp case above) and
    // New-Feature-ReflexiveProperty-001 (RL has no prp-rfx —
    // reflexive object property axioms are EXCLUDED from the RL grammar, Profiles
    // §4.2 — so no self-loop, and hence no reflexive-vs-irreflexive clash, may
    // appear in the closure).
    #[test]
    fn disjoint_property_fillers_do_not_get_a_differentfrom() {
        // :hasFather owl:propertyDisjointWith :hasMother ;
        // :Stewie :hasFather :Peter ; :Stewie :hasMother :Lois
        // Full OWL entails :Peter owl:differentFrom :Lois (the prp-pdw
        // CONTRAPOSITIVE), but NO OWL 2 RL/RDF rule derives differentFrom — the
        // RL profile is deliberately incomplete here. The closure must not contain
        // it (and the premise is consistent — different objects, so prp-pdw, which
        // needs the SAME (subject,object) pair, does not clash either).
        let mut dict = Dict::new();
        let (pdw, hf, hm, stewie, peter, lois) = (
            owl(&mut dict, "propertyDisjointWith"),
            ex(&mut dict, "hasFather"),
            ex(&mut dict, "hasMother"),
            ex(&mut dict, "Stewie"),
            ex(&mut dict, "Peter"),
            ex(&mut dict, "Lois"),
        );
        let different_from = owl(&mut dict, "differentFrom");
        let mut triples = vec![[hf, pdw, hm], [stewie, hf, peter], [stewie, hm, lois]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            !set.contains(&[peter, different_from, lois])
                && !set.contains(&[lois, different_from, peter]),
            "OWL 2 RL must NOT derive the prp-pdw contrapositive (differentFrom of the fillers)"
        );
        assert!(
            inconsistencies(&dict, &triples).is_empty(),
            "disjoint properties on DIFFERENT objects are consistent (prp-pdw needs the same pair)"
        );
    }

    #[test]
    fn functional_property_does_not_difference_distinct_subjects() {
        // :fp a owl:FunctionalProperty ; :Y2 :fp :X2 ; :Y1 :fp :X1 ;
        // :X1 owl:differentFrom :X2
        // Full OWL entails :Y1 owl:differentFrom :Y2 (the prp-fp CONTRAPOSITIVE),
        // but RL has no differentFrom-producing rule and prp-fp needs the SAME
        // subject (here Y1 ≠ Y2). The closure must derive neither a differentFrom
        // nor a (wrong) sameAs.
        let mut dict = Dict::new();
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let func = owl(&mut dict, "FunctionalProperty");
        let (fp, y1, y2, x1, x2) = (
            ex(&mut dict, "fp"),
            ex(&mut dict, "Y1"),
            ex(&mut dict, "Y2"),
            ex(&mut dict, "X1"),
            ex(&mut dict, "X2"),
        );
        let different_from = owl(&mut dict, "differentFrom");
        let same_as = owl(&mut dict, "sameAs");
        let mut triples = vec![
            [fp, ty, func],
            [y2, fp, x2],
            [y1, fp, x1],
            [x1, different_from, x2],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            !set.contains(&[y1, different_from, y2]) && !set.contains(&[y2, different_from, y1]),
            "OWL 2 RL must NOT derive the prp-fp contrapositive (differentFrom of the subjects)"
        );
        assert!(
            !set.contains(&[y1, same_as, y2]) && !set.contains(&[y2, same_as, y1]),
            "prp-fp must NOT fire across DISTINCT subjects"
        );
    }

    #[test]
    fn inverse_functional_property_does_not_difference_distinct_objects() {
        // :ifp a owl:InverseFunctionalProperty ; :X1 :ifp :Y1 ; :X2 :ifp :Y2 ;
        // :X1 owl:differentFrom :X2
        // Full OWL entails :Y1 owl:differentFrom :Y2 (the prp-ifp CONTRAPOSITIVE:
        // were Y1 = Y2, prp-ifp would merge X1/X2 against their differentFrom), but
        // RL has no differentFrom-producing rule and prp-ifp needs the SAME object
        // (here Y1 ≠ Y2). The closure must derive neither a differentFrom nor a
        // (wrong) sameAs, and must stay consistent.
        let mut dict = Dict::new();
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let inv_func = owl(&mut dict, "InverseFunctionalProperty");
        let (ifp, x1, x2, y1, y2) = (
            ex(&mut dict, "ifp"),
            ex(&mut dict, "X1"),
            ex(&mut dict, "X2"),
            ex(&mut dict, "Y1"),
            ex(&mut dict, "Y2"),
        );
        let different_from = owl(&mut dict, "differentFrom");
        let same_as = owl(&mut dict, "sameAs");
        let mut triples = vec![
            [ifp, ty, inv_func],
            [x1, ifp, y1],
            [x2, ifp, y2],
            [x1, different_from, x2],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            !set.contains(&[y1, different_from, y2]) && !set.contains(&[y2, different_from, y1]),
            "OWL 2 RL must NOT derive the prp-ifp contrapositive (differentFrom of the objects)"
        );
        assert!(
            !set.contains(&[y1, same_as, y2]) && !set.contains(&[y2, same_as, y1]),
            "prp-ifp must NOT fire across DISTINCT objects"
        );
        assert!(
            !set.contains(&[x1, same_as, x2]) && !set.contains(&[x2, same_as, x1]),
            "prp-ifp must NOT merge the subjects when their objects differ"
        );
        assert!(
            inconsistencies(&dict, &triples).is_empty(),
            "an IFP over DISTINCT objects with a differentFrom on the subjects is consistent"
        );
    }

    #[test]
    fn reflexive_property_self_loop_is_never_derived_nor_clashes() {
        // :p a owl:ReflexiveProperty, owl:IrreflexiveProperty ;
        // :q rdfs:subPropertyOf :p ; :a :q :b
        // Full OWL entails :a :p :a and :b :p :b (universal self-loop), which then
        // CLASHES with the irreflexivity of :p — full semantics is inconsistent.
        // OWL 2 RL excludes reflexive object property axioms from its grammar
        // (Profiles §4.2) and has NO prp-rfx rule, so the closure must contain no
        // self-loop and therefore prp-irp must report NO inconsistency. The
        // subPropertyOf edge is a positive control: spo1 still derives :a :p :b, so
        // a vacuous "nothing was materialized" run cannot pass this test. Extends
        // `reflexive_property_does_not_materialize_self_edges` past the
        // asserted-only case to a property reached through a DERIVED edge.
        let mut dict = Dict::new();
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let sp = dict.intern_iri(oxrdf::vocab::rdfs::SUB_PROPERTY_OF.as_str());
        let reflexive = owl(&mut dict, "ReflexiveProperty");
        let irreflexive = owl(&mut dict, "IrreflexiveProperty");
        let (p, q, a, b) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "q"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let mut triples = vec![
            [p, ty, reflexive],
            [p, ty, irreflexive],
            [q, sp, p],
            [a, q, b],
        ];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(
            set.contains(&[a, p, b]),
            "positive control: prp-spo1 must still lift :a :q :b to the super-property"
        );
        assert!(
            !set.contains(&[a, p, a]) && !set.contains(&[b, p, b]),
            "OWL 2 RL has no prp-rfx: a ReflexiveProperty must NOT self-loop any term"
        );
        assert!(
            !set.contains(&[a, q, a]) && !set.contains(&[b, q, b]),
            "no prp-rfx self-loop may leak down to the SUB-property either"
        );
        assert!(
            inconsistencies(&dict, &triples).is_empty(),
            "with no prp-rfx there is no self-loop for prp-irp to clash on, so RL reports \
             consistent where full OWL would not"
        );
    }

    // [OPUS-4.8] Regression for review 1402: an OWL feature predicate introduced via RDFS
    // subPropertyOf entailment (rdfs7) must NOT take the no-feature fast path — the derived
    // owl:sameAs must drive eq-rep substitution, not be emitted as an ordinary triple.
    #[test]
    fn subproperty_derived_sameas_runs_equality() {
        let mut dict = Dict::new();
        // :p rdfs:subPropertyOf owl:sameAs ; :a :p :b ; :b :likes :c
        // rdfs7 ⊢ (:a owl:sameAs :b); eq-rep then ⊢ (:a :likes :c).
        let (p, a, b, c, likes) = (
            ex(&mut dict, "p"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
            ex(&mut dict, "c"),
            ex(&mut dict, "likes"),
        );
        let sp = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subPropertyOf");
        let same_as = owl(&mut dict, "sameAs");
        let mut triples = vec![[p, sp, same_as], [a, p, b], [b, likes, c]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        // rdfs7 fires regardless of path:
        assert!(set.contains(&[a, same_as, b]), "rdfs7 should derive :a owl:sameAs :b");
        // The equality MUST be acted on: :a inherits :b's outgoing :likes :c (eq-rep-s).
        assert!(
            has(&dict, &set, "http://ex/a", "http://ex/likes", "http://ex/c"),
            "subproperty-derived owl:sameAs must drive eq-rep (1402)"
        );
    }

    // [OPUS-4.8] Regression for review 1402: an OWL feature CLASS introduced via RDFS subClassOf
    // entailment (rdfs9) must NOT take a fast path that drops the feature — a subclass of
    // owl:SymmetricProperty must yield symmetric reasoning on its instances.
    #[test]
    fn subclass_derived_symmetric_runs_symmetry() {
        let mut dict = Dict::new();
        // :MyProp rdfs:subClassOf owl:SymmetricProperty ; :knows a :MyProp ; :a :knows :b
        // rdfs9 ⊢ (:knows a owl:SymmetricProperty); prp-symp ⊢ (:b :knows :a).
        let (myprop, knows, a, b) = (
            ex(&mut dict, "MyProp"),
            ex(&mut dict, "knows"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let ty = dict.intern_iri(rdf::TYPE.as_str());
        let sc = dict.intern_iri("http://www.w3.org/2000/01/rdf-schema#subClassOf");
        let symmetric = owl(&mut dict, "SymmetricProperty");
        let mut triples = vec![[myprop, sc, symmetric], [knows, ty, myprop], [a, knows, b]];
        materialize_owl_rl(&mut dict, &mut triples);
        let set: FxHashSet<[Id; 3]> = triples.iter().copied().collect();
        assert!(set.contains(&[knows, ty, symmetric]), "rdfs9 should type :knows symmetric");
        assert!(
            set.contains(&[b, knows, a]),
            "subclass-derived owl:SymmetricProperty must drive prp-symp (1402)"
        );
    }

    // [OPUS-4.8] Guard: directly-asserted monotone features (inverseOf, equivalentClass) must STILL
    // take the monotone fast path — the non-reflexive RDFS-reachability check must not misfire on a
    // feature predicate that is merely *used* (only proper RDFS superproperties count).
    #[test]
    fn direct_monotone_features_unaffected() {
        let mut dict = Dict::new();
        let (parent, child, a, b) = (
            ex(&mut dict, "parentOf"),
            ex(&mut dict, "childOf"),
            ex(&mut dict, "a"),
            ex(&mut dict, "b"),
        );
        let inv = owl(&mut dict, "inverseOf");
        let v = Vocab::intern(&mut dict);
        let o = Owl::intern(&mut dict);
        let triples = vec![[parent, inv, child], [a, parent, b]];
        // inverseOf is a feature, so the fixpoint IS needed, but the monotone path must accept it.
        assert!(owl_uses_features(&triples, &v, &o), "inverseOf is a feature");
        assert!(
            monotone_only(&triples, &v, &o).is_some(),
            "direct inverseOf must remain on the monotone fast path"
        );
    }

    // [OPUS-4.8] sq-wtjol (sq-qcnn): RDF-list helper + targeted clash tests that pin the
    // list-PAIR-enumeration arithmetic in `inconsistencies()` (owl.rs §AllDisjointProperties /
    // §AllDisjointClasses / §AllDifferent). Each assertion is a hand-derived OWL-RL entailment
    // (prp-adp, cax-adc, eq-diff2/3 of the W3C OWL 2 RL/RDF rules), NOT a mere line-exercise:
    // the CONSISTENT variants prove the `i+1` upper-triangular offset and the strict pair
    // predicate (`a == b`, `count >= 2`) are EXACT — a self-pair (`ms[i..]`) or a flipped
    // comparison would manufacture a spurious clash that these `is_empty()` checks catch.

    /// Append an RDF collection `( m0 m1 … )` rooted at fresh blank-ish IRIs to `triples`,
    /// returning the head node id (or `rdf:nil` for the empty list). Mirrors the
    /// `rdf:first`/`rdf:rest`/`rdf:nil` shape that `inconsistencies()`'s `list()` walks.
    fn rdf_list(dict: &mut Dict, triples: &mut Vec<[Id; 3]>, members: &[Id]) -> Id {
        let first = rdf(dict, "first");
        let rest = rdf(dict, "rest");
        let nil = rdf(dict, "nil");
        let mut next = nil;
        // Build tail-first so each cons cell points at the already-constructed remainder.
        for (i, &m) in members.iter().enumerate().rev() {
            let cell = dict.intern_iri(&format!("http://ex/_lst{}_{}", members.len(), i));
            triples.push([cell, first, m]);
            triples.push([cell, rest, next]);
            next = cell;
        }
        next
    }

    #[test]
    fn all_disjoint_classes_three_members_pairwise() {
        // cax-adc over a 3-member AllDisjointClasses { :A :B :C }: an individual typed by
        // ANY TWO distinct members is a clash; one typed by a SINGLE member is consistent.
        // This pins `cs.iter().filter(...).count() >= 2` (a `>= → <` or `count` miscount
        // would flip both directions) and exercises the `list()` walk over 3 cells.
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let (adc_set, adc_ty, members) = (
            ex(&mut dict, "DisjClasses"),
            owl(&mut dict, "AllDisjointClasses"),
            owl(&mut dict, "members"),
        );
        let (ca, cb, cc) = (ex(&mut dict, "A"), ex(&mut dict, "B"), ex(&mut dict, "C"));
        let (x, y) = (ex(&mut dict, "x"), ex(&mut dict, "y"));
        let mut triples = vec![[adc_set, ty, adc_ty]];
        let head = rdf_list(&mut dict, &mut triples, &[ca, cb, cc]);
        triples.push([adc_set, members, head]);
        // :x typed by the FIRST and the LAST list members (a (0,2) pair, the offset the
        // mutated `i*1`/`i-1` arithmetic gets wrong) — a genuine cax-adc clash.
        triples.push([x, ty, ca]);
        triples.push([x, ty, cc]);
        // :y typed by exactly ONE member — must stay consistent.
        triples.push([y, ty, cb]);

        let clashes = inconsistencies(&dict, &triples);
        let x_iri = dict.term(x).to_string();
        let y_iri = dict.term(y).to_string();
        assert!(
            clashes.iter().any(|c| c.contains(&x_iri)),
            "cax-adc: :x typed by two AllDisjointClasses members is a clash; got {:?}",
            clashes
        );
        assert!(
            !clashes.iter().any(|c| c.contains(&y_iri)),
            "single-member :y must NOT be reported (no self-pair manufactured); got {:?}",
            clashes
        );
    }

    #[test]
    fn all_disjoint_classes_single_membership_consistent() {
        // A 3-member AllDisjointClasses where every individual is typed by AT MOST one
        // member is fully consistent. A self-pair bug (`>= 2` weakened, or a (i,i) pair)
        // would falsely report each singly-typed individual — this `is_empty()` forbids it.
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let (adc_set, adc_ty, members) = (
            ex(&mut dict, "DisjClasses"),
            owl(&mut dict, "AllDisjointClasses"),
            owl(&mut dict, "members"),
        );
        let (ca, cb, cc) = (ex(&mut dict, "A"), ex(&mut dict, "B"), ex(&mut dict, "C"));
        let (p, q, r) = (ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "r"));
        let mut triples = vec![[adc_set, ty, adc_ty]];
        let head = rdf_list(&mut dict, &mut triples, &[ca, cb, cc]);
        triples.push([adc_set, members, head]);
        triples.push([p, ty, ca]);
        triples.push([q, ty, cb]);
        triples.push([r, ty, cc]);
        assert!(
            inconsistencies(&dict, &triples).is_empty(),
            "disjoint classes with one-each membership is consistent"
        );
    }

    #[test]
    fn all_different_three_members_distinct_consistent() {
        // eq-diff2: a 3-member owl:AllDifferent of three DISTINCT individuals (no sameAs)
        // is consistent. This is the load-bearing negative for the AllDifferent pair loop:
        // a `xs[i..]` self-pair would fire `a == b`, and an `a == b → a != b` flip would
        // fire on every distinct pair — both would break this `is_empty()`.
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let (set, all_diff, members) = (
            ex(&mut dict, "Diff"),
            owl(&mut dict, "AllDifferent"),
            owl(&mut dict, "members"),
        );
        let (a, b, c) = (ex(&mut dict, "alice"), ex(&mut dict, "bob"), ex(&mut dict, "carol"));
        let mut triples = vec![[set, ty, all_diff]];
        let head = rdf_list(&mut dict, &mut triples, &[a, b, c]);
        triples.push([set, members, head]);
        assert!(
            inconsistencies(&dict, &triples).is_empty(),
            "three distinct AllDifferent members are consistent"
        );
    }

    #[test]
    fn all_different_sameas_pair_clashes() {
        // eq-diff3: a 3-member owl:AllDifferent whose FIRST and THIRD members are owl:sameAs
        // is inconsistent — the asserted distinctness is contradicted. Pins the `(0,2)`
        // upper-triangular pair (offset arithmetic) AND the `|| same.contains(..)` branch:
        // the same/sameAs check uses (min,max)-canonicalised pairs.
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let same_as = owl(&mut dict, "sameAs");
        let (set, all_diff, members) = (
            ex(&mut dict, "Diff"),
            owl(&mut dict, "AllDifferent"),
            owl(&mut dict, "members"),
        );
        let (a, b, c) = (ex(&mut dict, "p1"), ex(&mut dict, "p2"), ex(&mut dict, "p3"));
        let mut triples = vec![[set, ty, all_diff]];
        let head = rdf_list(&mut dict, &mut triples, &[a, b, c]);
        triples.push([set, members, head]);
        // p1 sameAs p3 — the (0,2) pair of a 3-element list.
        triples.push([a, same_as, c]);
        let clashes = inconsistencies(&dict, &triples);
        assert!(
            clashes.iter().any(|c| c.contains("are the same")),
            "AllDifferent members forced sameAs must clash; got {:?}",
            clashes
        );
    }

    #[test]
    fn all_different_distinct_members_predicate() {
        // eq-diff: owl:AllDifferent declared via owl:distinctMembers (the alternate
        // list predicate the loop also scans). Identical first/last ids are a clash.
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let (set, all_diff, distinct) = (
            ex(&mut dict, "Diff"),
            owl(&mut dict, "AllDifferent"),
            owl(&mut dict, "distinctMembers"),
        );
        let same_as = owl(&mut dict, "sameAs");
        let (a, b, c) = (ex(&mut dict, "q1"), ex(&mut dict, "q2"), ex(&mut dict, "q3"));
        let mut triples = vec![[set, ty, all_diff]];
        let head = rdf_list(&mut dict, &mut triples, &[a, b, c]);
        triples.push([set, distinct, head]);
        // Make the (1,2) pair sameAs to also pin a NON-(0,k) offset.
        triples.push([b, same_as, c]);
        let clashes = inconsistencies(&dict, &triples);
        assert!(
            clashes.iter().any(|c| c.contains("are the same")),
            "distinctMembers list with a sameAs pair clashes; got {:?}",
            clashes
        );
    }

    #[test]
    fn all_disjoint_properties_three_members_shared_pair() {
        // prp-adp over a 3-member owl:AllDisjointProperties { :p :q :r }: two DISTINCT
        // members sharing a (subject,object) pair is a clash; a single property used
        // alone is consistent. Pins the AllDisjointProperties pair-enumeration `ms[i+1..]`
        // (a `ms[i..]` self-pair would manufacture a (p,p) "disjoint" pair, falsely
        // flagging ANY used property as self-disjoint).
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let (set, adp_ty, members) = (
            ex(&mut dict, "DisjProps"),
            owl(&mut dict, "AllDisjointProperties"),
            owl(&mut dict, "members"),
        );
        let (p, q, r) = (ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "r"));
        let (s, o) = (ex(&mut dict, "s"), ex(&mut dict, "o"));
        let mut triples = vec![[set, ty, adp_ty]];
        let head = rdf_list(&mut dict, &mut triples, &[p, q, r]);
        triples.push([set, members, head]);
        // :s :p :o AND :s :r :o — p and r are the (0,2) disjoint pair sharing (s,o): clash.
        triples.push([s, p, o]);
        triples.push([s, r, o]);
        let clashes = inconsistencies(&dict, &triples);
        assert!(
            clashes.iter().any(|c| c.contains("disjoint properties")),
            "prp-adp: disjoint :p and :r sharing (s,o) is a clash; got {:?}",
            clashes
        );
    }

    #[test]
    fn all_disjoint_properties_single_use_consistent() {
        // A 3-member AllDisjointProperties where only ONE property is ever used, on a single
        // (s,o) pair, is consistent — no two DISTINCT disjoint properties share the pair.
        // The strict upper-triangular offset (`i+1`) must NOT pair :p with itself; this
        // `is_empty()` is exactly the negative that a `ms[i..]` self-pair mutant breaks.
        let mut dict = Dict::new();
        let ty = rdf(&mut dict, "type");
        let (set, adp_ty, members) = (
            ex(&mut dict, "DisjProps"),
            owl(&mut dict, "AllDisjointProperties"),
            owl(&mut dict, "members"),
        );
        let (p, q, r) = (ex(&mut dict, "p"), ex(&mut dict, "q"), ex(&mut dict, "r"));
        let (s, o) = (ex(&mut dict, "s"), ex(&mut dict, "o"));
        let mut triples = vec![[set, ty, adp_ty]];
        let head = rdf_list(&mut dict, &mut triples, &[p, q, r]);
        triples.push([set, members, head]);
        // Only :p is used. No two DISTINCT disjoint properties coincide.
        triples.push([s, p, o]);
        assert!(
            inconsistencies(&dict, &triples).is_empty(),
            "a single used disjoint property is not self-disjoint"
        );
    }
}
