// [GPT-6] Independent, deliberately unoptimized SPARQL 1.1 §18.4 oracle.
// It evaluates the published operators bottom-up without scan hints or SCCs.
use super::*;
use PropertyPathExpression as P;

fn specification(graph: &Graph, path: &P, x: Option<Id>, y: Option<Id>) -> Vec<(Id, Id)> {
    let nodes = || graph_nodes(graph);
    let mut result = match path {
        P::NamedNode(predicate) => {
            let pid = graph.id_of(&Term::NamedNode(predicate.clone()));
            let scan = graph.store.scan(&[None, None, None]);
            scan.rows
                .iter()
                .filter_map(|row| {
                    let triple = scan.to_spo(row);
                    (pid == Some(triple[1])).then_some((triple[0], triple[2]))
                })
                .collect()
        }
        P::Reverse(inner) => specification(graph, inner, y, x)
            .into_iter()
            .map(|(a, b)| (b, a))
            .collect(),
        P::Alternative(a, b) => {
            let mut result = specification(graph, a, x, y);
            result.extend(specification(graph, b, x, y));
            result
        }
        P::Sequence(a, b) => {
            let left = specification(graph, a, x, None);
            let right = specification(graph, b, None, y);
            let mut result = Vec::new();
            for &(start, mid) in &left {
                for &(other_mid, end) in &right {
                    if mid == other_mid {
                        result.push((start, end));
                    }
                }
            }
            result
        }
        P::ZeroOrOne(inner) => {
            let mut result: FxHashSet<_> = specification(graph, inner, x, y).into_iter().collect();
            if let Some(term) = x.or(y) {
                result.insert((term, term));
            } else {
                result.extend(nodes().into_iter().map(|n| (n, n)));
            }
            result.into_iter().collect()
        }
        P::ZeroOrMore(inner) | P::OneOrMore(inner) => {
            if x.is_none() && y.is_some() {
                let inverse = P::Reverse(inner.clone());
                let inverse = if matches!(path, P::ZeroOrMore(_)) {
                    P::ZeroOrMore(Box::new(inverse))
                } else {
                    P::OneOrMore(Box::new(inverse))
                };
                specification(graph, &inverse, y, None)
                    .into_iter()
                    .map(|(a, b)| (b, a))
                    .collect()
            } else {
                let starts = x.map_or_else(nodes, |x| FxHashSet::from_iter([x]));
                let mut result = Vec::new();
                for start in starts {
                    let mut pending: Vec<Id> = if matches!(path, P::ZeroOrMore(_)) {
                        vec![start]
                    } else {
                        specification(graph, inner, Some(start), None)
                            .into_iter()
                            .map(|(_, n)| n)
                            .collect()
                    };
                    let mut visited = FxHashSet::default();
                    while let Some(node) = pending.pop() {
                        if visited.insert(node) {
                            pending.extend(
                                specification(graph, inner, Some(node), None)
                                    .into_iter()
                                    .map(|(_, n)| n),
                            );
                        }
                    }
                    result.extend(visited.into_iter().map(|end| (start, end)));
                }
                result
            }
        }
        P::NegatedPropertySet(_) => {
            unreachable!("separate existing negated-set differential suite")
        }
    };
    result.retain(|&(a, b)| x.is_none_or(|x| a == x) && y.is_none_or(|y| b == y));
    result.sort_unstable();
    result
}

#[test]
fn nullable_hints_match_bottom_up_relational_specification() {
    let graph = Graph::load_str("@prefix :<http://ex/> . :a :p :b . :b :q :a .", "turtle").unwrap();
    let link = |p: &str| P::NamedNode(oxrdf::NamedNode::new(format!("http://ex/{p}")).unwrap());
    let base = vec![
        link("p"),
        link("q"),
        P::ZeroOrMore(Box::new(link("p"))),
        P::ZeroOrOne(Box::new(link("p"))),
        P::OneOrMore(Box::new(link("q"))),
    ];
    let mut paths = base.clone();
    for a in &base {
        paths.push(P::Reverse(Box::new(a.clone())));
        paths.push(P::ZeroOrMore(Box::new(a.clone())));
        paths.push(P::OneOrMore(Box::new(a.clone())));
        paths.push(P::ZeroOrOne(Box::new(a.clone())));
        for b in &base {
            let sequence = P::Sequence(Box::new(a.clone()), Box::new(b.clone()));
            paths.push(sequence.clone());
            paths.push(P::OneOrMore(Box::new(sequence)));
            paths.push(P::Alternative(Box::new(a.clone()), Box::new(b.clone())));
        }
    }
    let id = |name: &str| {
        graph
            .id_of(&Term::NamedNode(
                oxrdf::NamedNode::new(format!("http://ex/{name}")).unwrap(),
            ))
            .unwrap()
    };
    let mut local = LocalVocab::default();
    let missing = local.intern(Term::NamedNode(
        oxrdf::NamedNode::new("http://ex/missing").unwrap(),
    ));
    // Includes a graph node, predicate-only dictionary term and absent local term.
    let endpoints = [None, Some(id("a")), Some(id("p")), Some(missing)];
    for path in paths {
        for x in endpoints {
            for y in endpoints {
                let expected = specification(&graph, &path, x, y);
                for hint in endpoints {
                    let mut ends = PathEnds::terms(x, y);
                    if x.is_none() {
                        ends.s = hint;
                    }
                    if y.is_none() {
                        ends.o = hint;
                    }
                    let restrict = |&(a, b): &(Id, Id)| {
                        ends.s.is_none_or(|s| s == a) && ends.o.is_none_or(|o| o == b)
                    };
                    let want: Vec<_> = expected.iter().copied().filter(restrict).collect();
                    let mut got: Vec<_> = path_bag_pairs(&graph, &path, ends)
                        .unwrap()
                        .into_iter()
                        .filter(restrict)
                        .collect();
                    got.sort_unstable();
                    assert_eq!(got, want, "{path:?}, x={x:?}, y={y:?}, hint={hint:?}");
                }
            }
        }
    }
}

#[test]
fn path_multiplicity_overflow_fails_before_allocation() {
    assert_eq!(
        check_path_growth(usize::MAX, 1).unwrap_err(),
        "property path multiplicity overflow"
    );
}
