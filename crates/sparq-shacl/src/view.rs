//! A term-level read view over a dictionary-encoded [`sparq_core::Graph`].
//!
//! SHACL evaluation is term-oriented (focus nodes, value nodes, shape parameters
//! are RDF terms), so this wraps the id-level permutation scans of the store and
//! materialises `oxrdf::Term`s at the boundary. Every lookup is index-backed
//! (bound-prefix range scan on the best permutation), never a full-graph filter.

use oxrdf::{NamedNode, Term};
use sparq_core::dict::Id;
use sparq_core::Graph;

pub(crate) const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub(crate) const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
pub(crate) const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
pub(crate) const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";
pub(crate) const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

/// A borrowed, term-level view of a graph.
#[derive(Clone, Copy)]
pub struct GraphView<'g> {
    g: &'g Graph,
}

impl<'g> GraphView<'g> {
    pub fn new(g: &'g Graph) -> Self {
        GraphView { g }
    }

    /// The underlying graph — the seam SHACL-SPARQL (`sh:sparql`) uses to run its
    /// `sh:select` query through `sparq-engine`.
    pub fn graph(&self) -> &'g Graph {
        self.g
    }

    /// All triples matching the optional-term pattern, as `[s, p, o]` terms.
    /// A bound term absent from the graph's dictionary matches nothing.
    pub fn triples(&self, s: Option<&Term>, p: Option<&str>, o: Option<&Term>) -> Vec<[Term; 3]> {
        let p_node = match p {
            None => None,
            Some(iri) => match NamedNode::new(iri) {
                Ok(n) => Some(n),
                Err(_) => return Vec::new(),
            },
        };
        let Some(pattern) = self.g.pattern(s, p_node.as_ref(), o) else {
            return Vec::new();
        };
        let scan = self.g.store().scan(&pattern);
        scan.rows
            .iter()
            .map(|r| {
                let [s, p, o] = scan.to_spo(r);
                [
                    self.g.dict().term(s),
                    self.g.dict().term(p),
                    self.g.dict().term(o),
                ]
            })
            .collect()
    }

    // ---- id-level twins (sq-7d3dj.33.4) --------------------------------
    // The Term-level lookups above pay a dictionary hash of every bound term PLUS an
    // `oxrdf::Term` materialisation per matched row on EVERY call — the profiled hot
    // cost of core-constraint validation (find_iri/hash_iri + Term alloc churn per
    // focus node). These twins keep the whole per-focus walk at the `Id` level and
    // materialise a `Term` only at the report boundary. Row order is identical to
    // the Term-level methods (same permutation scans).

    /// The dictionary id of `t`, or `None` when absent (matches nothing).
    pub(crate) fn id_of(&self, t: &Term) -> Option<Id> {
        self.g.id_of(t)
    }

    /// The id of a predicate IRI string (`None`: invalid IRI or absent from the
    /// dictionary — either way the predicate matches nothing, exactly like the
    /// Term-level `triples` guard).
    pub(crate) fn pred_id(&self, iri: &str) -> Option<Id> {
        let n = NamedNode::new(iri).ok()?;
        self.g.id_of(&Term::NamedNode(n))
    }

    /// Materialises the term for `id` (the report boundary).
    pub(crate) fn term_of(&self, id: Id) -> Term {
        self.g.dict().term(id)
    }

    /// Object ids of `(s, p, ?)` — id twin of [`Self::objects`], same row order.
    pub(crate) fn objects_ids(&self, s: Id, p: Id) -> Vec<Id> {
        let scan = self.g.store().scan(&[Some(s), Some(p), None]);
        scan.rows.iter().map(|r| scan.to_spo(r)[2]).collect()
    }

    /// Subject ids of `(?, p, o)` — id twin of [`Self::subjects`], same row order.
    pub(crate) fn subjects_ids(&self, p: Id, o: Id) -> Vec<Id> {
        let scan = self.g.store().scan(&[None, Some(p), Some(o)]);
        scan.rows.iter().map(|r| scan.to_spo(r)[0]).collect()
    }

    /// `(predicate, object)` id pairs of triples with subject `s` — id twin of
    /// [`Self::predicate_objects`] (`sh:closed`), same row order. (sq-j9hls)
    pub(crate) fn predicate_objects_ids(&self, s: Id) -> Vec<(Id, Id)> {
        let scan = self.g.store().scan(&[Some(s), None, None]);
        scan.rows
            .iter()
            .map(|r| {
                let [_, p, o] = scan.to_spo(r);
                (p, o)
            })
            .collect()
    }

    /// True iff the fully-ground triple is present.
    pub fn contains(&self, s: &Term, p: &str, o: &Term) -> bool {
        let Ok(p) = NamedNode::new(p) else {
            return false;
        };
        match self.g.pattern(Some(s), Some(&p), Some(o)) {
            Some([Some(s), Some(p), Some(o)]) => self.g.store().contains([s, p, o]),
            _ => false,
        }
    }

    pub fn objects(&self, s: &Term, p: &str) -> Vec<Term> {
        self.triples(Some(s), Some(p), None)
            .into_iter()
            .map(|[_, _, o]| o)
            .collect()
    }

    pub fn object(&self, s: &Term, p: &str) -> Option<Term> {
        self.objects(s, p).into_iter().next()
    }

    pub fn subjects(&self, p: &str, o: &Term) -> Vec<Term> {
        self.triples(None, Some(p), Some(o))
            .into_iter()
            .map(|[s, _, _]| s)
            .collect()
    }

    /// Distinct subjects of any triple with predicate `p`.
    pub fn subjects_of(&self, p: &str) -> Vec<Term> {
        dedup(
            self.triples(None, Some(p), None)
                .into_iter()
                .map(|[s, _, _]| s),
        )
    }

    /// Distinct objects of any triple with predicate `p`.
    pub fn objects_of(&self, p: &str) -> Vec<Term> {
        dedup(
            self.triples(None, Some(p), None)
                .into_iter()
                .map(|[_, _, o]| o),
        )
    }

    /// All `(predicate, object)` pairs of triples with subject `s` (sh:closed).
    pub fn predicate_objects(&self, s: &Term) -> Vec<(Term, Term)> {
        self.triples(Some(s), None, None)
            .into_iter()
            .map(|[_, p, o]| (p, o))
            .collect()
    }

    /// The lexical form of the first literal object, if any.
    pub fn str_object(&self, s: &Term, p: &str) -> Option<String> {
        match self.object(s, p) {
            Some(Term::Literal(l)) => Some(l.value().to_string()),
            _ => None,
        }
    }

    /// True iff `s` has `rdf:type o`.
    pub fn has_type(&self, s: &Term, class: &str) -> bool {
        match NamedNode::new(class) {
            Ok(c) => self.contains(s, RDF_TYPE, &Term::NamedNode(c)),
            Err(_) => false,
        }
    }

    /// Walks an `rdf:first`/`rdf:rest` list from `head` (cycle-safe).
    pub fn list(&self, head: &Term) -> Vec<Term> {
        let mut out = Vec::new();
        let mut seen: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
        let mut cur = head.clone();
        loop {
            if matches!(&cur, Term::NamedNode(n) if n.as_str() == RDF_NIL) {
                break;
            }
            if !seen.insert(cur.clone()) {
                break; // malformed cyclic list
            }
            match self.object(&cur, RDF_FIRST) {
                Some(f) => out.push(f),
                None => break, // not a (well-formed) list node
            }
            match self.object(&cur, RDF_REST) {
                Some(r) => cur = r,
                None => break,
            }
        }
        out
    }

    /// (sq-vg3y) The members of `head` iff it is a WELL-FORMED SHACL
    /// list (SHACL §1.4 "SHACL Lists"), else `None`. A SHACL list is `rdf:nil`
    /// (with no `rdf:first`/`rdf:rest` — empty list), or a node with EXACTLY ONE
    /// `rdf:first`, EXACTLY ONE `rdf:rest` that is itself a SHACL list, and that
    /// does not reach itself via `rdf:rest+` (no cycle). A literal head, a missing
    /// or duplicated `rdf:first`/`rdf:rest`, or a cycle ⇒ NOT a SHACL list. This is
    /// the strict check `sh:memberShape` / `sh:{min,max}ListLength` /
    /// `sh:uniqueMembers` need (the lenient [`Self::list`] tolerates malformed
    /// tails, which would silently truncate those constraints).
    pub fn list_strict(&self, head: &Term) -> Option<Vec<Term>> {
        if matches!(head, Term::Literal(_)) {
            return None; // a literal is never a SHACL list
        }
        let mut out = Vec::new();
        let mut seen: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
        let mut cur = head.clone();
        loop {
            if matches!(&cur, Term::NamedNode(n) if n.as_str() == RDF_NIL) {
                // rdf:nil must carry no rdf:first/rdf:rest to be the empty list.
                if self.object(&cur, RDF_FIRST).is_some() || self.object(&cur, RDF_REST).is_some() {
                    return None;
                }
                return Some(out);
            }
            if !seen.insert(cur.clone()) {
                return None; // cycle via rdf:rest+
            }
            // Exactly one rdf:first and exactly one rdf:rest.
            let firsts = self.objects(&cur, RDF_FIRST);
            let rests = self.objects(&cur, RDF_REST);
            if firsts.len() != 1 || rests.len() != 1 {
                return None;
            }
            out.push(firsts.into_iter().next().unwrap());
            cur = rests.into_iter().next().unwrap();
        }
    }

    /// The SHACL instances of `class`: nodes whose `rdf:type` is `class` or any
    /// `rdfs:subClassOf`-descendant of it (closure computed within this graph).
    pub fn instances_of(&self, class: &Term) -> Vec<Term> {
        let mut out = Vec::new();
        let mut seen: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
        for c in self.subclass_closure(class) {
            for s in self.subjects(RDF_TYPE, &c) {
                if seen.insert(s.clone()) {
                    out.push(s);
                }
            }
        }
        out
    }

    /// `class` plus all its `rdfs:subClassOf`-descendants (reflexive-transitive).
    pub fn subclass_closure(&self, class: &Term) -> Vec<Term> {
        let mut out = vec![class.clone()];
        let mut seen: rustc_hash::FxHashSet<Term> = std::iter::once(class.clone()).collect();
        let mut i = 0;
        while i < out.len() {
            let c = out[i].clone();
            for sub in self.subjects(RDFS_SUBCLASS_OF, &c) {
                if seen.insert(sub.clone()) {
                    out.push(sub);
                }
            }
            i += 1;
        }
        out
    }

    /// True iff `node` is a SHACL instance of `class` in this graph.
    pub fn is_instance_of(&self, node: &Term, class: &Term) -> bool {
        if matches!(node, Term::Literal(_)) {
            return false;
        }
        for ty in self.objects(node, RDF_TYPE) {
            if &ty == class {
                return true;
            }
            // Walk ty's superclasses looking for `class`.
            let mut stack = vec![ty];
            let mut seen: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
            while let Some(t) = stack.pop() {
                if !seen.insert(t.clone()) {
                    continue;
                }
                for sup in self.objects(&t, RDFS_SUBCLASS_OF) {
                    if &sup == class {
                        return true;
                    }
                    stack.push(sup);
                }
            }
        }
        false
    }
}

/// Order-preserving dedup.
pub(crate) fn dedup(items: impl IntoIterator<Item = Term>) -> Vec<Term> {
    let mut seen: rustc_hash::FxHashSet<Term> = rustc_hash::FxHashSet::default();
    let mut out = Vec::new();
    for t in items {
        if seen.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

/// Order-preserving dedup over ids — the id twin of [`dedup`]. Ids and terms are
/// in bijection (the dictionary is injective), so deduplicating ids keeps exactly
/// the elements (and order) the Term-level dedup would. (sq-7d3dj.33.4)
pub(crate) fn dedup_ids(items: impl IntoIterator<Item = Id>) -> Vec<Id> {
    let mut seen: rustc_hash::FxHashSet<Id> = rustc_hash::FxHashSet::default();
    let mut out = Vec::new();
    for id in items {
        if seen.insert(id) {
            out.push(id);
        }
    }
    out
}

// (sq-bif) Direct unit coverage for the public `GraphView` read-view.
// The W3C-suite harness drives `view.object`/`view.list` indirectly, but the
// edge-case branches of this term-level API — the invalid-IRI guards, the
// cycle-safe lenient `list` vs the strict `list_strict` (which must REJECT a
// malformed/duplicated/cyclic list rather than silently truncate), and the
// `rdfs:subClassOf` closure walk in `instances_of`/`is_instance_of` — are
// load-bearing and were previously unexercised at the assertion level. Graphs are
// built term-exactly via `crate::graph_from_triples` so malformed lists, cycles
// and subclass hierarchies can be constructed precisely.
#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Triple};

    const EX: &str = "http://example.org/";

    fn nn(s: &str) -> NamedNode {
        NamedNode::new_unchecked(s.to_string())
    }
    fn iri(s: &str) -> Term {
        Term::NamedNode(nn(s))
    }
    fn ex(local: &str) -> Term {
        iri(&format!("{EX}{local}"))
    }
    fn lit(s: &str) -> Term {
        Term::Literal(Literal::new_simple_literal(s))
    }

    /// Build a graph from `(subject, predicate, object)` term triples. A literal
    /// subject is illegal in RDF and is skipped (no such triple in our fixtures).
    fn graph(triples: &[(Term, &str, Term)]) -> Graph {
        let ts: Vec<Triple> = triples
            .iter()
            .filter_map(|(s, p, o)| {
                let subj = match s {
                    Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
                    Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
                    // A literal (or RDF-star quoted-triple) subject is illegal in
                    // RDF 1.1 and never appears in our fixtures — skip it.
                    _ => return None,
                };
                Some(Triple::new(subj, nn(p), o.clone()))
            })
            .collect();
        crate::graph_from_triples(ts)
    }

    // ---- pattern scans + invalid-IRI guards ----

    #[test]
    fn triples_invalid_predicate_iri_matches_nothing() {
        let g = graph(&[(ex("s"), &format!("{EX}p"), ex("o"))]);
        let v = GraphView::new(&g);
        // A syntactically invalid predicate IRI (contains a space) cannot name a
        // term, so the scan returns nothing rather than erroring.
        assert!(v.triples(None, Some("not a valid iri"), None).is_empty());
        // A well-formed but ABSENT predicate also matches nothing (dict miss).
        assert!(v
            .triples(None, Some(&format!("{EX}absent")), None)
            .is_empty());
        // The real predicate matches the one triple.
        assert_eq!(v.triples(None, Some(&format!("{EX}p")), None).len(), 1);
    }

    #[test]
    fn contains_respects_ground_terms_and_invalid_iri() {
        let g = graph(&[(ex("s"), &format!("{EX}p"), ex("o"))]);
        let v = GraphView::new(&g);
        assert!(v.contains(&ex("s"), &format!("{EX}p"), &ex("o")));
        // Wrong object — not present.
        assert!(!v.contains(&ex("s"), &format!("{EX}p"), &ex("other")));
        // A subject term absent from the dictionary — not present.
        assert!(!v.contains(&ex("ghost"), &format!("{EX}p"), &ex("o")));
        // An invalid predicate IRI — false, not a panic.
        assert!(!v.contains(&ex("s"), "bad iri", &ex("o")));
    }

    // ---- objects / subjects / dedup projections ----

    #[test]
    fn object_objects_subjects_and_str_object() {
        let g = graph(&[
            (ex("s"), &format!("{EX}p"), ex("a")),
            (ex("s"), &format!("{EX}p"), ex("b")),
            (ex("s"), &format!("{EX}label"), lit("hi")),
            (ex("t"), &format!("{EX}p"), ex("a")),
        ]);
        let v = GraphView::new(&g);
        // objects: both a and b (order not guaranteed → compare as a set).
        let mut objs: Vec<String> = v
            .objects(&ex("s"), &format!("{EX}p"))
            .iter()
            .map(|t| t.to_string())
            .collect();
        objs.sort();
        assert_eq!(objs, vec![ex("a").to_string(), ex("b").to_string()]);
        // object: the first (some) value of p; absent predicate → None.
        assert!(v.object(&ex("s"), &format!("{EX}p")).is_some());
        assert!(v.object(&ex("s"), &format!("{EX}absent")).is_none());
        // subjects of (p, a): both s and t.
        let mut subs: Vec<String> = v
            .subjects(&format!("{EX}p"), &ex("a"))
            .iter()
            .map(|t| t.to_string())
            .collect();
        subs.sort();
        assert_eq!(subs, vec![ex("s").to_string(), ex("t").to_string()]);
        // str_object: the lexical form of a literal object; None for an IRI object.
        assert_eq!(
            v.str_object(&ex("s"), &format!("{EX}label")),
            Some("hi".to_string())
        );
        assert_eq!(v.str_object(&ex("s"), &format!("{EX}p")), None);
    }

    #[test]
    fn subjects_of_and_objects_of_dedup() {
        // Two subjects each with the predicate twice over distinct objects: the
        // DISTINCT subjects of p is {s, t}; the DISTINCT objects is {a, b, c}.
        let g = graph(&[
            (ex("s"), &format!("{EX}p"), ex("a")),
            (ex("s"), &format!("{EX}p"), ex("b")),
            (ex("t"), &format!("{EX}p"), ex("b")),
            (ex("t"), &format!("{EX}p"), ex("c")),
        ]);
        let v = GraphView::new(&g);
        assert_eq!(v.subjects_of(&format!("{EX}p")).len(), 2);
        assert_eq!(v.objects_of(&format!("{EX}p")).len(), 3);
    }

    #[test]
    fn predicate_objects_lists_every_pair() {
        let g = graph(&[
            (ex("s"), &format!("{EX}p"), ex("a")),
            (ex("s"), &format!("{EX}q"), lit("x")),
        ]);
        let v = GraphView::new(&g);
        let mut pairs: Vec<(String, String)> = v
            .predicate_objects(&ex("s"))
            .into_iter()
            .map(|(p, o)| (p.to_string(), o.to_string()))
            .collect();
        pairs.sort();
        assert_eq!(pairs.len(), 2);
        assert!(pairs.iter().any(|(p, _)| p.ends_with("q>")));
    }

    // ---- has_type / instances_of / subclass closure ----

    #[test]
    fn has_type_direct_only_and_invalid_class() {
        let g = graph(&[(ex("alice"), RDF_TYPE, ex("Person"))]);
        let v = GraphView::new(&g);
        assert!(v.has_type(&ex("alice"), &format!("{EX}Person")));
        // has_type is direct (no subclass walk); a superclass is NOT a direct type.
        assert!(!v.has_type(&ex("alice"), &format!("{EX}Agent")));
        // An invalid class IRI → false, not a panic.
        assert!(!v.has_type(&ex("alice"), "not an iri"));
    }

    #[test]
    fn instances_of_walks_subclass_closure() {
        // Worker ⊑ Employee ⊑ Person. An instance of the most-derived class is an
        // instance of every ancestor.
        let g = graph(&[
            (ex("Employee"), RDFS_SUBCLASS_OF, ex("Person")),
            (ex("Worker"), RDFS_SUBCLASS_OF, ex("Employee")),
            (ex("alice"), RDF_TYPE, ex("Worker")),
            (ex("bob"), RDF_TYPE, ex("Employee")),
            (ex("carol"), RDF_TYPE, ex("Person")),
        ]);
        let v = GraphView::new(&g);
        // instances_of(Person) = closure {Person, Employee, Worker} typed nodes.
        let mut insts: Vec<String> = v
            .instances_of(&ex("Person"))
            .iter()
            .map(|t| t.to_string())
            .collect();
        insts.sort();
        assert_eq!(
            insts,
            vec![
                ex("alice").to_string(),
                ex("bob").to_string(),
                ex("carol").to_string()
            ]
        );
        // instances_of(Employee) excludes the Person-only carol.
        let mut insts: Vec<String> = v
            .instances_of(&ex("Employee"))
            .iter()
            .map(|t| t.to_string())
            .collect();
        insts.sort();
        assert_eq!(insts, vec![ex("alice").to_string(), ex("bob").to_string()]);
    }

    #[test]
    fn instances_of_dedupes_across_subclass_routes() {
        // A node typed BOTH as the class and a subclass must appear once.
        let g = graph(&[
            (ex("Sub"), RDFS_SUBCLASS_OF, ex("Super")),
            (ex("x"), RDF_TYPE, ex("Super")),
            (ex("x"), RDF_TYPE, ex("Sub")),
        ]);
        let v = GraphView::new(&g);
        assert_eq!(v.instances_of(&ex("Super")).len(), 1);
    }

    #[test]
    fn subclass_closure_is_cycle_safe_and_reflexive() {
        // A subclass CYCLE (A ⊑ B ⊑ A) must not loop forever; the closure is the
        // reflexive set {A, B}.
        let g = graph(&[
            (ex("A"), RDFS_SUBCLASS_OF, ex("B")),
            (ex("B"), RDFS_SUBCLASS_OF, ex("A")),
        ]);
        let v = GraphView::new(&g);
        let mut clo: Vec<String> = v
            .subclass_closure(&ex("A"))
            .iter()
            .map(|t| t.to_string())
            .collect();
        clo.sort();
        assert_eq!(clo, vec![ex("A").to_string(), ex("B").to_string()]);
        // A class with no declared subclasses → just itself (reflexive).
        assert_eq!(v.subclass_closure(&ex("Lonely")).len(), 1);
    }

    #[test]
    fn is_instance_of_walks_superclasses_and_rejects_literals() {
        let g = graph(&[
            (ex("Employee"), RDFS_SUBCLASS_OF, ex("Person")),
            (ex("alice"), RDF_TYPE, ex("Employee")),
        ]);
        let v = GraphView::new(&g);
        // Direct type.
        assert!(v.is_instance_of(&ex("alice"), &ex("Employee")));
        // Via the rdfs:subClassOf walk (Employee ⊑ Person).
        assert!(v.is_instance_of(&ex("alice"), &ex("Person")));
        // Not an instance of an unrelated class.
        assert!(!v.is_instance_of(&ex("alice"), &ex("Robot")));
        // A literal is never a SHACL instance of any class (early return).
        assert!(!v.is_instance_of(&lit("x"), &ex("Person")));
    }

    #[test]
    fn is_instance_of_handles_superclass_cycle() {
        // A ⊑ B ⊑ A with x typed A: the superclass walk must terminate.
        let g = graph(&[
            (ex("A"), RDFS_SUBCLASS_OF, ex("B")),
            (ex("B"), RDFS_SUBCLASS_OF, ex("A")),
            (ex("x"), RDF_TYPE, ex("A")),
        ]);
        let v = GraphView::new(&g);
        assert!(v.is_instance_of(&ex("x"), &ex("A")));
        assert!(v.is_instance_of(&ex("x"), &ex("B")));
        assert!(!v.is_instance_of(&ex("x"), &ex("Unrelated")));
    }

    // ---- list (lenient) vs list_strict (well-formed only) ----

    /// Build an `rdf:first`/`rdf:rest` chain over the given members rooted at a
    /// fresh blank node; the tail is `rdf:nil`. Returns (head, triples).
    fn well_formed_list(members: &[Term]) -> (Term, Vec<(Term, &'static str, Term)>) {
        let nil = iri(RDF_NIL);
        let mut triples: Vec<(Term, &'static str, Term)> = Vec::new();
        let mut next = nil.clone();
        // Build right-to-left so each cons cell points at the previously-built tail.
        for m in members.iter().rev() {
            let cell = Term::BlankNode(BlankNode::default());
            triples.push((cell.clone(), RDF_FIRST, m.clone()));
            triples.push((cell.clone(), RDF_REST, next.clone()));
            next = cell;
        }
        (next, triples)
    }

    #[test]
    fn list_and_list_strict_well_formed() {
        let (head, ts) = well_formed_list(&[ex("a"), ex("b"), ex("c")]);
        let g = graph(&ts);
        let v = GraphView::new(&g);
        let want = vec![ex("a"), ex("b"), ex("c")];
        assert_eq!(v.list(&head), want);
        assert_eq!(v.list_strict(&head), Some(want));
        // The empty list: rdf:nil with nothing attached.
        let empty = graph(&[(ex("s"), &format!("{EX}p"), ex("o"))]);
        let ev = GraphView::new(&empty);
        assert!(ev.list(&iri(RDF_NIL)).is_empty());
        assert_eq!(ev.list_strict(&iri(RDF_NIL)), Some(vec![]));
    }

    #[test]
    fn list_strict_rejects_literal_head() {
        let g = graph(&[(ex("s"), &format!("{EX}p"), ex("o"))]);
        let v = GraphView::new(&g);
        // A literal is never a SHACL list.
        assert_eq!(v.list_strict(&lit("x")), None);
        // The lenient walk also yields nothing for a literal head (no rdf:first).
        assert!(v.list(&lit("x")).is_empty());
    }

    #[test]
    fn list_strict_rejects_nil_with_extra_triples() {
        // rdf:nil carrying an rdf:first is NOT the empty list under the strict check.
        let g = graph(&[(iri(RDF_NIL), RDF_FIRST, ex("a"))]);
        let v = GraphView::new(&g);
        assert_eq!(v.list_strict(&iri(RDF_NIL)), None);
    }

    #[test]
    fn list_strict_rejects_duplicate_first_or_rest() {
        // A cons cell with TWO rdf:first values is malformed → rejected.
        let cell = Term::BlankNode(BlankNode::default());
        let g = graph(&[
            (cell.clone(), RDF_FIRST, ex("a")),
            (cell.clone(), RDF_FIRST, ex("b")),
            (cell.clone(), RDF_REST, iri(RDF_NIL)),
        ]);
        let v = GraphView::new(&g);
        assert_eq!(v.list_strict(&cell), None);
        // But the lenient `list` still extracts ONE member (it tolerates the
        // malformation) — exactly the silent-truncation `list_strict` guards against.
        assert_eq!(v.list(&cell).len(), 1);
    }

    #[test]
    fn list_strict_rejects_missing_first() {
        // A cell with only rdf:rest (no rdf:first) is not a well-formed list node.
        let cell = Term::BlankNode(BlankNode::default());
        let g = graph(&[(cell.clone(), RDF_REST, iri(RDF_NIL))]);
        let v = GraphView::new(&g);
        assert_eq!(v.list_strict(&cell), None);
        // Lenient `list` breaks at the missing rdf:first → empty.
        assert!(v.list(&cell).is_empty());
    }

    #[test]
    fn list_and_list_strict_reject_cycle() {
        // A cell whose rdf:rest points back at itself: a cycle via rdf:rest+.
        let cell = Term::BlankNode(BlankNode::default());
        let g = graph(&[
            (cell.clone(), RDF_FIRST, ex("a")),
            (cell.clone(), RDF_REST, cell.clone()),
        ]);
        let v = GraphView::new(&g);
        // list_strict rejects a cycle outright.
        assert_eq!(v.list_strict(&cell), None);
        // The lenient `list` is cycle-SAFE: it terminates, yielding the members
        // seen before re-entry (here the single 'a') rather than looping forever.
        assert_eq!(v.list(&cell), vec![ex("a")]);
    }

    #[test]
    fn list_lenient_truncates_malformed_tail() {
        // head -> a -> (cell with no rdf:rest): lenient `list` returns [a, b] then
        // stops; list_strict rejects because the second cell has no rdf:rest.
        let c1 = Term::BlankNode(BlankNode::default());
        let c2 = Term::BlankNode(BlankNode::default());
        let g = graph(&[
            (c1.clone(), RDF_FIRST, ex("a")),
            (c1.clone(), RDF_REST, c2.clone()),
            (c2.clone(), RDF_FIRST, ex("b")),
            // c2 has NO rdf:rest → malformed tail.
        ]);
        let v = GraphView::new(&g);
        assert_eq!(v.list(&c1), vec![ex("a"), ex("b")]);
        assert_eq!(v.list_strict(&c1), None);
    }

    #[test]
    fn graph_accessor_returns_underlying() {
        let g = graph(&[(ex("s"), &format!("{EX}p"), ex("o"))]);
        let v = GraphView::new(&g);
        // The seam SHACL-SPARQL uses: the accessor hands back the same graph.
        assert!(std::ptr::eq(v.graph(), &g));
    }
}
