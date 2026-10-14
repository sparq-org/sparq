//! SHACL property paths: the parsed form, evaluation by direct graph walks, and
//! serialisation back to a Turtle path expression for validation reports.

use crate::view::{dedup, dedup_ids, GraphView, RDF_FIRST};
use oxrdf::Term;
use rustc_hash::FxHashSet;
use sparq_core::dict::Id;

const SH: &str = "http://www.w3.org/ns/shacl#";

/// A SHACL property path (sh:path), structurally faithful to the shapes graph so
/// reports can serialise the same expression back out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Path {
    /// A predicate path: the IRI of the property.
    Predicate(String),
    Inverse(Box<Path>),
    Sequence(Vec<Path>),
    Alternative(Vec<Path>),
    ZeroOrMore(Box<Path>),
    OneOrMore(Box<Path>),
    ZeroOrOne(Box<Path>),
}

impl Path {
    /// Parses a path expression rooted at `node` in a shapes graph.
    pub fn parse(g: &GraphView, node: &Term) -> Result<Path, String> {
        Self::parse_guarded(g, node, &mut FxHashSet::default())
    }

    fn parse_guarded(
        g: &GraphView,
        node: &Term,
        seen: &mut FxHashSet<Term>,
    ) -> Result<Path, String> {
        match node {
            Term::NamedNode(n) => Ok(Path::Predicate(n.as_str().to_string())),
            Term::BlankNode(_) => {
                // Chain-based cycle guard: a node may legitimately occur twice as a
                // SIBLING (e.g. the sequence `( _:inv _:inv )`); only re-entry on the
                // CURRENT parse chain is a cycle, so the node is removed again after
                // its subtree parses (see the `seen.remove` below).
                if !seen.insert(node.clone()) {
                    return Err("cyclic path expression".into());
                }
                let parsed = Self::parse_blank(g, node, seen);
                seen.remove(node);
                parsed
            }
            Term::Literal(_) => Err("literal in path expression".into()),
            #[allow(unreachable_patterns)]
            _ => Err("unsupported path term".into()),
        }
    }

    fn parse_blank(g: &GraphView, node: &Term, seen: &mut FxHashSet<Term>) -> Result<Path, String> {
        // An rdf:list head is a sequence path.
        if g.object(node, RDF_FIRST).is_some() {
            let items = g.list(node);
            if items.len() < 2 {
                return Err("sequence path with fewer than two members".into());
            }
            let parts = items
                .iter()
                .map(|i| Self::parse_guarded(g, i, seen))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Path::Sequence(parts));
        }
        if let Some(p) = g.object(node, &format!("{SH}inversePath")) {
            return Ok(Path::Inverse(Box::new(Self::parse_guarded(g, &p, seen)?)));
        }
        if let Some(p) = g.object(node, &format!("{SH}alternativePath")) {
            let items = g.list(&p);
            if items.len() < 2 {
                return Err("alternative path with fewer than two members".into());
            }
            let parts = items
                .iter()
                .map(|i| Self::parse_guarded(g, i, seen))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Path::Alternative(parts));
        }
        if let Some(p) = g.object(node, &format!("{SH}zeroOrMorePath")) {
            return Ok(Path::ZeroOrMore(Box::new(Self::parse_guarded(
                g, &p, seen,
            )?)));
        }
        if let Some(p) = g.object(node, &format!("{SH}oneOrMorePath")) {
            return Ok(Path::OneOrMore(Box::new(Self::parse_guarded(g, &p, seen)?)));
        }
        if let Some(p) = g.object(node, &format!("{SH}zeroOrOnePath")) {
            return Ok(Path::ZeroOrOne(Box::new(Self::parse_guarded(g, &p, seen)?)));
        }
        Err("ill-formed path blank node".into())
    }

    /// The value nodes reachable from `start` along this path (a set, in
    /// discovery order — SHACL value nodes are distinct).
    pub fn values(&self, g: &GraphView, start: &Term) -> Vec<Term> {
        self.step(g, start, true)
    }

    /// (sq-7d3dj.33.4) Compiles this path against `g`'s dictionary:
    /// every predicate IRI is resolved to its id ONCE, so the per-focus-node walk
    /// ([`PathIds::values_ids`]) never re-hashes a predicate string. A predicate
    /// that is invalid or absent from the dictionary compiles to `None` — that
    /// step matches nothing, exactly like the Term-level `triples` guard.
    pub(crate) fn compile(&self, g: &GraphView) -> PathIds {
        match self {
            Path::Predicate(p) => PathIds::Predicate(g.pred_id(p)),
            Path::Inverse(inner) => PathIds::Inverse(Box::new(inner.compile(g))),
            Path::Sequence(parts) => {
                PathIds::Sequence(parts.iter().map(|p| p.compile(g)).collect())
            }
            Path::Alternative(parts) => {
                PathIds::Alternative(parts.iter().map(|p| p.compile(g)).collect())
            }
            Path::ZeroOrMore(inner) => PathIds::ZeroOrMore(Box::new(inner.compile(g))),
            Path::OneOrMore(inner) => PathIds::OneOrMore(Box::new(inner.compile(g))),
            Path::ZeroOrOne(inner) => PathIds::ZeroOrOne(Box::new(inner.compile(g))),
        }
    }

    /// One application of the path from `node`; `forward` flips under inversion.
    fn step(&self, g: &GraphView, node: &Term, forward: bool) -> Vec<Term> {
        match self {
            Path::Predicate(p) => {
                if forward {
                    g.objects(node, p)
                } else {
                    g.subjects(p, node)
                }
            }
            Path::Inverse(inner) => inner.step(g, node, !forward),
            Path::Sequence(parts) => {
                let mut nodes = vec![node.clone()];
                let order: Vec<&Path> = if forward {
                    parts.iter().collect()
                } else {
                    parts.iter().rev().collect()
                };
                for p in order {
                    nodes = dedup(nodes.iter().flat_map(|n| p.step(g, n, forward)));
                    if nodes.is_empty() {
                        break;
                    }
                }
                nodes
            }
            Path::Alternative(parts) => dedup(parts.iter().flat_map(|p| p.step(g, node, forward))),
            Path::ZeroOrOne(inner) => {
                dedup(std::iter::once(node.clone()).chain(inner.step(g, node, forward)))
            }
            Path::ZeroOrMore(inner) => closure(g, node, inner, forward, true),
            Path::OneOrMore(inner) => closure(g, node, inner, forward, false),
        }
    }

    /// Serialises the path as a Turtle path expression usable as the object of
    /// `sh:resultPath` (predicate IRIs, `( … )` sequences, `[ sh:inversePath … ]`
    /// etc. for the other forms).
    pub fn to_turtle(&self) -> String {
        match self {
            Path::Predicate(p) => format!("<{p}>"),
            Path::Inverse(p) => format!("[ <{SH}inversePath> {} ]", p.to_turtle()),
            Path::Sequence(ps) => {
                let inner: Vec<String> = ps.iter().map(|p| p.to_turtle()).collect();
                format!("( {} )", inner.join(" "))
            }
            Path::Alternative(ps) => {
                let inner: Vec<String> = ps.iter().map(|p| p.to_turtle()).collect();
                format!("[ <{SH}alternativePath> ( {} ) ]", inner.join(" "))
            }
            Path::ZeroOrMore(p) => format!("[ <{SH}zeroOrMorePath> {} ]", p.to_turtle()),
            Path::OneOrMore(p) => format!("[ <{SH}oneOrMorePath> {} ]", p.to_turtle()),
            Path::ZeroOrOne(p) => format!("[ <{SH}zeroOrOnePath> {} ]", p.to_turtle()),
        }
    }

    /// Serialises the path as a **SPARQL property-path expression**
    /// (for `$PATH` pre-binding in SHACL-SPARQL property constraints, §5.2.1):
    /// `<iri>`, `^p`, `p1/p2`, `p1|p2`, `p*`, `p+`, `p?`, fully parenthesised so
    /// it composes safely when textually spliced into a query's BGP. Returns
    /// `None` for an empty/degenerate sequence or alternative (no SPARQL form).
    pub fn to_sparql_property_path(&self) -> Option<String> {
        Some(match self {
            Path::Predicate(p) => format!("<{p}>"),
            Path::Inverse(p) => format!("^({})", p.to_sparql_property_path()?),
            Path::Sequence(ps) => {
                if ps.is_empty() {
                    return None;
                }
                let inner: Vec<String> = ps
                    .iter()
                    .map(|p| p.to_sparql_property_path())
                    .collect::<Option<_>>()?;
                format!("({})", inner.join("/"))
            }
            Path::Alternative(ps) => {
                if ps.is_empty() {
                    return None;
                }
                let inner: Vec<String> = ps
                    .iter()
                    .map(|p| p.to_sparql_property_path())
                    .collect::<Option<_>>()?;
                format!("({})", inner.join("|"))
            }
            Path::ZeroOrMore(p) => format!("({})*", p.to_sparql_property_path()?),
            Path::OneOrMore(p) => format!("({})+", p.to_sparql_property_path()?),
            Path::ZeroOrOne(p) => format!("({})?", p.to_sparql_property_path()?),
        })
    }
}

/// (sq-7d3dj.33.4) The id-level compiled form of a [`Path`]: predicate
/// IRIs pre-resolved to dictionary ids (`None` = invalid/absent → matches
/// nothing). [`PathIds::values_ids`] mirrors [`Path::values`] step-for-step over
/// the same permutation scans, so the id walk yields exactly the ids of the
/// terms the Term-level walk yields, in the same discovery order — the
/// result-equivalence the id fast path rests on (differential-tested in
/// `lib.rs::idfast_equivalence`).
#[derive(Debug, Clone)]
pub(crate) enum PathIds {
    Predicate(Option<Id>),
    Inverse(Box<PathIds>),
    Sequence(Vec<PathIds>),
    Alternative(Vec<PathIds>),
    ZeroOrMore(Box<PathIds>),
    OneOrMore(Box<PathIds>),
    ZeroOrOne(Box<PathIds>),
}

impl PathIds {
    /// The value-node ids reachable from `start` along this path (a set, in
    /// discovery order) — the id twin of [`Path::values`].
    pub(crate) fn values_ids(&self, g: &GraphView, start: Id) -> Vec<Id> {
        self.step_ids(g, start, true)
    }

    /// One application of the path from `node` — the id twin of [`Path::step`].
    fn step_ids(&self, g: &GraphView, node: Id, forward: bool) -> Vec<Id> {
        match self {
            PathIds::Predicate(p) => match p {
                None => Vec::new(),
                Some(p) => {
                    if forward {
                        g.objects_ids(node, *p)
                    } else {
                        g.subjects_ids(*p, node)
                    }
                }
            },
            PathIds::Inverse(inner) => inner.step_ids(g, node, !forward),
            PathIds::Sequence(parts) => {
                let mut nodes = vec![node];
                let order: Vec<&PathIds> = if forward {
                    parts.iter().collect()
                } else {
                    parts.iter().rev().collect()
                };
                for p in order {
                    nodes = dedup_ids(nodes.iter().flat_map(|&n| p.step_ids(g, n, forward)));
                    if nodes.is_empty() {
                        break;
                    }
                }
                nodes
            }
            PathIds::Alternative(parts) => {
                dedup_ids(parts.iter().flat_map(|p| p.step_ids(g, node, forward)))
            }
            PathIds::ZeroOrOne(inner) => {
                dedup_ids(std::iter::once(node).chain(inner.step_ids(g, node, forward)))
            }
            PathIds::ZeroOrMore(inner) => closure_ids(g, node, inner, forward, true),
            PathIds::OneOrMore(inner) => closure_ids(g, node, inner, forward, false),
        }
    }
}

/// Breadth-first reachability closure over ids — the id twin of [`closure`].
fn closure_ids(g: &GraphView, start: Id, inner: &PathIds, forward: bool, reflexive: bool) -> Vec<Id> {
    let mut seen: FxHashSet<Id> = FxHashSet::default();
    let mut out: Vec<Id> = Vec::new();
    let mut queue: Vec<Id> = vec![start];
    if reflexive {
        seen.insert(start);
        out.push(start);
    }
    let mut i = 0;
    while i < queue.len() {
        let n = queue[i];
        i += 1;
        for next in inner.step_ids(g, n, forward) {
            if seen.insert(next) {
                out.push(next);
                queue.push(next);
            }
        }
    }
    out
}

/// Breadth-first reachability closure of `inner` from `start`; `reflexive`
/// includes `start` itself (zeroOrMore vs oneOrMore).
fn closure(g: &GraphView, start: &Term, inner: &Path, forward: bool, reflexive: bool) -> Vec<Term> {
    let mut seen: FxHashSet<Term> = FxHashSet::default();
    let mut out: Vec<Term> = Vec::new();
    let mut queue: Vec<Term> = vec![start.clone()];
    if reflexive {
        seen.insert(start.clone());
        out.push(start.clone());
    }
    let mut i = 0;
    while i < queue.len() {
        let n = queue[i].clone();
        i += 1;
        for next in inner.step(g, &n, forward) {
            if seen.insert(next.clone()) {
                out.push(next.clone());
                queue.push(next);
            }
        }
    }
    out
}

// Unit coverage for SHACL property paths (sq-qap0): parsing every
// path form (and its error branches), value-node evaluation (step + closure)
// with hand-computed expectations, and both serialisations (Turtle path
// expression + SPARQL property path). path.rs was the lowest-covered file in the
// crate (~36%); these exercise the parse/eval/serialise surface directly.
#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{NamedNode, Term};
    use sparq_core::Graph;

    const EX: &str = "http://example.org/";

    /// IRI term in the ex: namespace.
    fn n(local: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(format!("{EX}{local}")))
    }

    /// The IRI string in the ex: namespace (for `Path::Predicate`).
    fn p(local: &str) -> String {
        format!("{EX}{local}")
    }

    /// Loads a Turtle shapes/data graph (the leak of the `Graph` keeps the
    /// `GraphView` borrow simple in each test).
    fn graph(ttl: &str) -> Graph {
        let doc = format!("@prefix ex: <{EX}> .\n@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n{ttl}");
        Graph::load_str(&doc, "turtle").unwrap()
    }

    /// Parses the path rooted at the object of `ex:root ex:path ?o` in a shapes
    /// graph, returning the parsed `Path` (the path-expression blank/IRI tree is
    /// declared as `ex:root sh:path <expr>`).
    fn parse_path(shapes_ttl: &str) -> Result<Path, String> {
        let g = graph(shapes_ttl);
        let view = GraphView::new(&g);
        let root = n("root");
        let path_obj = view
            .object(&root, "http://www.w3.org/ns/shacl#path")
            .expect("ex:root sh:path <expr>");
        Path::parse(&view, &path_obj)
    }

    /// `path.values(view, start)` as a set, sorted by string for stable
    /// comparison (value-node order is discovery order, which we don't assert).
    fn values_set(g: &Graph, path: &Path, start: &Term) -> Vec<String> {
        let view = GraphView::new(g);
        let mut v: Vec<String> = path
            .values(&view, start)
            .iter()
            .map(|t| t.to_string())
            .collect();
        v.sort();
        v
    }

    fn names(locals: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = locals.iter().map(|l| n(l).to_string()).collect();
        v.sort();
        v
    }

    // ---- parsing: every form + error branches ----

    #[test]
    fn parse_predicate_path() {
        let path = parse_path("ex:root sh:path ex:knows .").unwrap();
        assert_eq!(path, Path::Predicate(p("knows")));
    }

    #[test]
    fn parse_inverse_path() {
        let path = parse_path("ex:root sh:path [ sh:inversePath ex:parent ] .").unwrap();
        assert_eq!(path, Path::Inverse(Box::new(Path::Predicate(p("parent")))));
    }

    #[test]
    fn parse_sequence_path() {
        let path = parse_path("ex:root sh:path ( ex:a ex:b ex:c ) .").unwrap();
        assert_eq!(
            path,
            Path::Sequence(vec![
                Path::Predicate(p("a")),
                Path::Predicate(p("b")),
                Path::Predicate(p("c")),
            ])
        );
    }

    #[test]
    fn parse_alternative_path() {
        let path = parse_path("ex:root sh:path [ sh:alternativePath ( ex:a ex:b ) ] .").unwrap();
        assert_eq!(
            path,
            Path::Alternative(vec![Path::Predicate(p("a")), Path::Predicate(p("b"))])
        );
    }

    #[test]
    fn parse_recursive_forms() {
        assert_eq!(
            parse_path("ex:root sh:path [ sh:zeroOrMorePath ex:p ] .").unwrap(),
            Path::ZeroOrMore(Box::new(Path::Predicate(p("p"))))
        );
        assert_eq!(
            parse_path("ex:root sh:path [ sh:oneOrMorePath ex:p ] .").unwrap(),
            Path::OneOrMore(Box::new(Path::Predicate(p("p"))))
        );
        assert_eq!(
            parse_path("ex:root sh:path [ sh:zeroOrOnePath ex:p ] .").unwrap(),
            Path::ZeroOrOne(Box::new(Path::Predicate(p("p"))))
        );
    }

    #[test]
    fn parse_nested_path() {
        // ( [inverse ex:a] [zeroOrMore ex:b] ) — a sequence of non-predicate parts.
        let path =
            parse_path("ex:root sh:path ( [ sh:inversePath ex:a ] [ sh:zeroOrMorePath ex:b ] ) .")
                .unwrap();
        assert_eq!(
            path,
            Path::Sequence(vec![
                Path::Inverse(Box::new(Path::Predicate(p("a")))),
                Path::ZeroOrMore(Box::new(Path::Predicate(p("b")))),
            ])
        );
    }

    #[test]
    fn parse_literal_in_path_is_error() {
        let g = graph("ex:root sh:path 42 .");
        let view = GraphView::new(&g);
        let obj = view
            .object(&n("root"), "http://www.w3.org/ns/shacl#path")
            .unwrap();
        assert!(matches!(obj, Term::Literal(_)));
        assert_eq!(
            Path::parse(&view, &obj).unwrap_err(),
            "literal in path expression"
        );
    }

    #[test]
    fn parse_sequence_too_short_is_error() {
        // A one-element rdf:list is not a valid sequence path.
        let err = parse_path("ex:root sh:path ( ex:a ) .").unwrap_err();
        assert_eq!(err, "sequence path with fewer than two members");
    }

    #[test]
    fn parse_alternative_too_short_is_error() {
        let err = parse_path("ex:root sh:path [ sh:alternativePath ( ex:a ) ] .").unwrap_err();
        assert_eq!(err, "alternative path with fewer than two members");
    }

    #[test]
    fn parse_ill_formed_blank_is_error() {
        // A blank node that is neither a list head nor any sh:*Path wrapper.
        let err = parse_path("ex:root sh:path [ ex:notAPathPredicate ex:x ] .").unwrap_err();
        assert_eq!(err, "ill-formed path blank node");
    }

    #[test]
    fn parse_cyclic_path_is_error() {
        // A blank node whose inversePath points back at itself: re-entry on the
        // current chain is a cycle.
        let g = graph("ex:root sh:path _:b . _:b sh:inversePath _:b .");
        let view = GraphView::new(&g);
        let obj = view
            .object(&n("root"), "http://www.w3.org/ns/shacl#path")
            .unwrap();
        assert_eq!(
            Path::parse(&view, &obj).unwrap_err(),
            "cyclic path expression"
        );
    }

    #[test]
    fn parse_sibling_reuse_is_not_cyclic() {
        // The SAME inverse-path blank node used twice as siblings in a sequence
        // is legal (the chain guard removes it after each subtree) — regression
        // for the documented `( _:inv _:inv )` case.
        let g = graph("ex:root sh:path ( _:inv _:inv ) . _:inv sh:inversePath ex:p .");
        let view = GraphView::new(&g);
        let obj = view
            .object(&n("root"), "http://www.w3.org/ns/shacl#path")
            .unwrap();
        let path = Path::parse(&view, &obj).unwrap();
        let inv = Path::Inverse(Box::new(Path::Predicate(p("p"))));
        assert_eq!(path, Path::Sequence(vec![inv.clone(), inv]));
    }

    // ---- evaluation: hand-computed value sets, positive + negative ----

    #[test]
    fn eval_predicate_forward_and_empty() {
        let g = graph("ex:a ex:knows ex:b , ex:c .");
        let path = Path::Predicate(p("knows"));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["b", "c"]));
        // No outgoing ex:knows from ex:b — empty (negative).
        assert!(values_set(&g, &path, &n("b")).is_empty());
    }

    #[test]
    fn eval_inverse() {
        // ex:b ex:parent ex:a , ex:c ; inverse(parent) from ex:a yields {ex:b}.
        let g = graph("ex:b ex:parent ex:a . ex:d ex:parent ex:a .");
        let path = Path::Inverse(Box::new(Path::Predicate(p("parent"))));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["b", "d"]));
        // No one has ex:c as a parent value — empty.
        assert!(values_set(&g, &path, &n("c")).is_empty());
    }

    #[test]
    fn eval_sequence_direction_matters() {
        // a -knows-> b -name-> "Bob"; ( ex:knows ex:name ) from a yields {"Bob"}.
        let g = graph(r#"ex:a ex:knows ex:b . ex:b ex:name "Bob" ."#);
        let path = Path::Sequence(vec![
            Path::Predicate(p("knows")),
            Path::Predicate(p("name")),
        ]);
        let view = GraphView::new(&g);
        let vals = path.values(&view, &n("a"));
        assert_eq!(vals.len(), 1);
        assert!(matches!(&vals[0], Term::Literal(l) if l.value() == "Bob"));
        // A sequence where the first step dead-ends: empty (the early break).
        assert!(path.values(&view, &n("b")).is_empty());
    }

    #[test]
    fn eval_sequence_inverse_first_step() {
        // ( [inverse ex:parent] ex:name ): from ex:a, step inverse(parent) ->
        // {ex:b}, then ex:name -> {"B"}. Exercises the inverse-inside-sequence
        // forward/!forward interplay.
        let g = graph(r#"ex:b ex:parent ex:a . ex:b ex:name "B" ."#);
        let path = Path::Sequence(vec![
            Path::Inverse(Box::new(Path::Predicate(p("parent")))),
            Path::Predicate(p("name")),
        ]);
        let view = GraphView::new(&g);
        let vals: Vec<String> = path
            .values(&view, &n("a"))
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(vals, vec![r#""B""#.to_string()]);
    }

    #[test]
    fn eval_alternative_dedups() {
        // a -p1-> b ; a -p2-> b , c. Alternative(p1,p2) from a = {b, c} (b once).
        let g = graph("ex:a ex:p1 ex:b . ex:a ex:p2 ex:b , ex:c .");
        let path = Path::Alternative(vec![Path::Predicate(p("p1")), Path::Predicate(p("p2"))]);
        let view = GraphView::new(&g);
        let vals = path.values(&view, &n("a"));
        assert_eq!(vals.len(), 2, "duplicate ex:b must be collapsed");
        assert_eq!(values_set(&g, &path, &n("a")), names(&["b", "c"]));
    }

    #[test]
    fn eval_zero_or_one_includes_start() {
        // a -p-> b. ZeroOrOne(p) from a = {a, b}; from a leaf c = {c}.
        let g = graph("ex:a ex:p ex:b .");
        let path = Path::ZeroOrOne(Box::new(Path::Predicate(p("p"))));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["a", "b"]));
        assert_eq!(values_set(&g, &path, &n("c")), names(&["c"]));
    }

    #[test]
    fn eval_zero_or_more_reflexive_and_transitive() {
        // Chain a -p-> b -p-> c -p-> d. ZeroOrMore(p) from a = {a,b,c,d}.
        let g = graph("ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:d .");
        let path = Path::ZeroOrMore(Box::new(Path::Predicate(p("p"))));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["a", "b", "c", "d"]));
        // From a node with no successors: just itself (reflexive).
        assert_eq!(values_set(&g, &path, &n("d")), names(&["d"]));
    }

    #[test]
    fn eval_one_or_more_excludes_start_unless_reachable() {
        // Same chain. OneOrMore(p) from a = {b,c,d} (NOT a — non-reflexive).
        let g = graph("ex:a ex:p ex:b . ex:b ex:p ex:c . ex:c ex:p ex:d .");
        let path = Path::OneOrMore(Box::new(Path::Predicate(p("p"))));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["b", "c", "d"]));
        // A leaf with no successors yields nothing under oneOrMore.
        assert!(values_set(&g, &path, &n("d")).is_empty());
    }

    #[test]
    fn eval_one_or_more_includes_start_in_a_cycle() {
        // A cycle a -p-> b -p-> a. OneOrMore(p) from a reaches b AND back to a,
        // so a IS in the result (reachable in >=1 step). The closure's seen-set
        // must terminate the cycle.
        let g = graph("ex:a ex:p ex:b . ex:b ex:p ex:a .");
        let path = Path::OneOrMore(Box::new(Path::Predicate(p("p"))));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["a", "b"]));
    }

    #[test]
    fn eval_zero_or_more_inverse() {
        // ZeroOrMore over an inverse path: with b -p-> a, c -p-> b, the inverse
        // closure from a = {a, b, c}.
        let g = graph("ex:b ex:p ex:a . ex:c ex:p ex:b .");
        let path = Path::ZeroOrMore(Box::new(Path::Inverse(Box::new(Path::Predicate(p("p"))))));
        assert_eq!(values_set(&g, &path, &n("a")), names(&["a", "b", "c"]));
    }

    #[test]
    fn eval_inverse_of_sequence_reverses_order() {
        // ^(ex:a/ex:b) from t = { s : s -a-> m -b-> t }. The inverse must walk
        // the sequence in REVERSE (b then a, each step inverted) — exercises the
        // `parts.iter().rev()` + !forward interplay in `step`.
        let g = graph("ex:s ex:a ex:m . ex:m ex:b ex:t .");
        let path = Path::Inverse(Box::new(Path::Sequence(vec![
            Path::Predicate(p("a")),
            Path::Predicate(p("b")),
        ])));
        assert_eq!(values_set(&g, &path, &n("t")), names(&["s"]));
        // From a node with no inbound a/b walk: empty.
        assert!(values_set(&g, &path, &n("m")).is_empty());
    }

    #[test]
    fn eval_alternative_with_inverse_branch() {
        // (ex:a | ^ex:b) from t: forward a -> {u}; inverse b -> {s} (s -b-> t).
        let g = graph("ex:t ex:a ex:u . ex:s ex:b ex:t .");
        let path = Path::Alternative(vec![
            Path::Predicate(p("a")),
            Path::Inverse(Box::new(Path::Predicate(p("b")))),
        ]);
        assert_eq!(values_set(&g, &path, &n("t")), names(&["s", "u"]));
    }

    #[test]
    fn eval_zero_or_more_of_sequence() {
        // (ex:a/ex:b)* from x over x -a-> m1 -b-> y -a-> m2 -b-> z: the closure's
        // inner step is a two-hop sequence, so the reachable set is {x, y, z}
        // (the intermediate m1/m2 are NOT value nodes of the composite step).
        let g = graph("ex:x ex:a ex:m1 . ex:m1 ex:b ex:y . ex:y ex:a ex:m2 . ex:m2 ex:b ex:z .");
        let path = Path::ZeroOrMore(Box::new(Path::Sequence(vec![
            Path::Predicate(p("a")),
            Path::Predicate(p("b")),
        ])));
        assert_eq!(values_set(&g, &path, &n("x")), names(&["x", "y", "z"]));
    }

    #[test]
    fn eval_nested_sequence_with_star() {
        // ( ex:friend [zeroOrMore ex:friend] ): from a, ex:friend -> {b}, then
        // zeroOrMore(friend) from b. With b -friend-> c -friend-> d:
        // result = {b, c, d}.
        let g = graph("ex:a ex:friend ex:b . ex:b ex:friend ex:c . ex:c ex:friend ex:d .");
        let path = Path::Sequence(vec![
            Path::Predicate(p("friend")),
            Path::ZeroOrMore(Box::new(Path::Predicate(p("friend")))),
        ]);
        assert_eq!(values_set(&g, &path, &n("a")), names(&["b", "c", "d"]));
    }

    // ---- serialisation: Turtle path expression ----

    #[test]
    fn to_turtle_round_trips_each_form() {
        let cases = [
            (Path::Predicate(p("knows")), format!("<{EX}knows>")),
            (
                Path::Inverse(Box::new(Path::Predicate(p("p")))),
                format!("[ <{SH}inversePath> <{EX}p> ]"),
            ),
            (
                Path::Sequence(vec![Path::Predicate(p("a")), Path::Predicate(p("b"))]),
                format!("( <{EX}a> <{EX}b> )"),
            ),
            (
                Path::Alternative(vec![Path::Predicate(p("a")), Path::Predicate(p("b"))]),
                format!("[ <{SH}alternativePath> ( <{EX}a> <{EX}b> ) ]"),
            ),
            (
                Path::ZeroOrMore(Box::new(Path::Predicate(p("p")))),
                format!("[ <{SH}zeroOrMorePath> <{EX}p> ]"),
            ),
            (
                Path::OneOrMore(Box::new(Path::Predicate(p("p")))),
                format!("[ <{SH}oneOrMorePath> <{EX}p> ]"),
            ),
            (
                Path::ZeroOrOne(Box::new(Path::Predicate(p("p")))),
                format!("[ <{SH}zeroOrOnePath> <{EX}p> ]"),
            ),
        ];
        for (path, expect) in cases {
            assert_eq!(path.to_turtle(), expect, "to_turtle for {path:?}");
        }
    }

    /// The Turtle of a path expression must re-parse to the same `Path` when
    /// placed back as a `sh:path` object.
    #[test]
    fn to_turtle_reparses_to_same_path() {
        let path = Path::Sequence(vec![
            Path::Inverse(Box::new(Path::Predicate(p("a")))),
            Path::Alternative(vec![
                Path::Predicate(p("b")),
                Path::ZeroOrMore(Box::new(Path::Predicate(p("c")))),
            ]),
        ]);
        let g = graph(&format!("ex:root sh:path {} .", path.to_turtle()));
        let view = GraphView::new(&g);
        let obj = view
            .object(&n("root"), "http://www.w3.org/ns/shacl#path")
            .unwrap();
        assert_eq!(Path::parse(&view, &obj).unwrap(), path);
    }

    // ---- serialisation: SPARQL property path ----

    #[test]
    fn to_sparql_property_path_each_form() {
        assert_eq!(
            Path::Predicate(p("a")).to_sparql_property_path().unwrap(),
            format!("<{EX}a>")
        );
        assert_eq!(
            Path::Inverse(Box::new(Path::Predicate(p("a"))))
                .to_sparql_property_path()
                .unwrap(),
            format!("^(<{EX}a>)")
        );
        assert_eq!(
            Path::Sequence(vec![Path::Predicate(p("a")), Path::Predicate(p("b"))])
                .to_sparql_property_path()
                .unwrap(),
            format!("(<{EX}a>/<{EX}b>)")
        );
        assert_eq!(
            Path::Alternative(vec![Path::Predicate(p("a")), Path::Predicate(p("b"))])
                .to_sparql_property_path()
                .unwrap(),
            format!("(<{EX}a>|<{EX}b>)")
        );
        assert_eq!(
            Path::ZeroOrMore(Box::new(Path::Predicate(p("a"))))
                .to_sparql_property_path()
                .unwrap(),
            format!("(<{EX}a>)*")
        );
        assert_eq!(
            Path::OneOrMore(Box::new(Path::Predicate(p("a"))))
                .to_sparql_property_path()
                .unwrap(),
            format!("(<{EX}a>)+")
        );
        assert_eq!(
            Path::ZeroOrOne(Box::new(Path::Predicate(p("a"))))
                .to_sparql_property_path()
                .unwrap(),
            format!("(<{EX}a>)?")
        );
    }

    /// (sq-7d3dj.33.4) The compiled id-level walk must yield EXACTLY the
    /// terms of the Term-level walk, in the SAME discovery order (the report-
    /// equivalence the eval fast path rests on), across every path form —
    /// including an absent predicate (compiles to a matches-nothing step) and a
    /// knows-cycle through the id closure.
    #[test]
    fn values_ids_mirror_values_order_exactly() {
        let g = graph(
            "ex:a ex:knows ex:b , ex:c . ex:b ex:knows ex:a . ex:c ex:knows ex:d .
             ex:b ex:name \"B\" . ex:d ex:name \"D\" .
             ex:x ex:parent ex:a . ex:y ex:parent ex:a .",
        );
        let view = GraphView::new(&g);
        let paths = [
            Path::Predicate(p("knows")),
            Path::Predicate(p("absent")),
            Path::Inverse(Box::new(Path::Predicate(p("parent")))),
            Path::Sequence(vec![Path::Predicate(p("knows")), Path::Predicate(p("name"))]),
            Path::Alternative(vec![Path::Predicate(p("knows")), Path::Predicate(p("parent"))]),
            Path::ZeroOrMore(Box::new(Path::Predicate(p("knows")))),
            Path::OneOrMore(Box::new(Path::Predicate(p("knows")))),
            Path::ZeroOrOne(Box::new(Path::Predicate(p("knows")))),
            Path::Inverse(Box::new(Path::Sequence(vec![
                Path::Predicate(p("knows")),
                Path::Predicate(p("name")),
            ]))),
        ];
        for path in &paths {
            let compiled = path.compile(&view);
            for start in ["a", "b", "d", "x"] {
                let start_term = n(start);
                let by_terms = path.values(&view, &start_term);
                let start_id = view.id_of(&start_term).expect("start interned");
                let by_ids: Vec<Term> = compiled
                    .values_ids(&view, start_id)
                    .into_iter()
                    .map(|id| view.term_of(id))
                    .collect();
                assert_eq!(
                    by_ids, by_terms,
                    "id walk diverged for {path:?} from ex:{start}"
                );
            }
        }
    }

    #[test]
    fn to_sparql_property_path_none_for_empty() {
        // Degenerate empty sequence / alternative have no SPARQL form.
        assert_eq!(Path::Sequence(vec![]).to_sparql_property_path(), None);
        assert_eq!(Path::Alternative(vec![]).to_sparql_property_path(), None);
        // A degenerate child propagates None through a wrapper.
        assert_eq!(
            Path::ZeroOrMore(Box::new(Path::Sequence(vec![]))).to_sparql_property_path(),
            None
        );
        assert_eq!(
            Path::Inverse(Box::new(Path::Alternative(vec![]))).to_sparql_property_path(),
            None
        );
    }
}
