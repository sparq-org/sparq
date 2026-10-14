//! Which reasoning-input documents each access-control document's grants can depend on, and
//! the reverse (issue #5312; `research/solid-pod-scoped-materializer-design.md` §5.2).
//!
//! The materializer reasons over one fact pool: every control document of the system plus,
//! for WAC, every group document an `acl:agentGroup` names, together with the loader's
//! structural facts. [`input_docs`] attributes that pool to its documents through the same
//! per-triple path the loader uses ([`input_fact`]: `solidx:` facts dropped, blank nodes
//! skolemized), so the index sees the reasoner's identities, and a control document also
//! owns its structural `<R> solidx:ownAcl|ownAcr <doc>` and `solidx:inDoc` facts. A
//! document's grants can therefore change when another document
//!
//! - defines a node this one links to through a [`LINKAGE`] or structural predicate (an
//!   ACP control naming a policy or matcher defined elsewhere, a WAC group whose members a
//!   group document lists, a document describing this control document's own IRI), or
//! - states facts about a subject this one also describes (two documents sharing an
//!   authorization or policy IRI, case (C) of the design record, including a resource IRI
//!   and a skolemized blank node).
//!
//! [`ControlDeps::build`] follows both edge kinds to a fixpoint. It only derives state; no
//! materialization reads it yet. The trusted creator/owner and credential facts are not
//! document content and are outside the index.

use crate::loader::{
    graph_iri, graph_triples, group_doc_of, in_doc_predicate, input_fact, is_authority_less,
    own_predicate, referenced_group_docs, System, ACL_AGENT_GROUP, ACL_SUFFIX, ACR_SUFFIX,
    RESERVED_PREFIX,
};
use oxrdf::{NamedNode, Term};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::Graph;

/// The document predicates whose object a rule body binds to a variable, so the object can
/// join a fact from another document. `linkage_covers_every_rule_body_predicate` parses
/// `rules/*.n3` and fails when a rule binds the object of a predicate missing here.
pub(crate) const LINKAGE: &[&str] = &[
    "http://www.w3.org/ns/auth/acl#accessTo",
    "http://www.w3.org/ns/auth/acl#default",
    "http://www.w3.org/ns/auth/acl#agent",
    "http://www.w3.org/ns/auth/acl#agentGroup",
    "http://www.w3.org/ns/auth/acl#origin",
    "http://www.w3.org/ns/auth/acl#mode",
    "http://www.w3.org/2006/vcard/ns#hasMember",
    "http://www.w3.org/ns/solid/acp#accessControl",
    "http://www.w3.org/ns/solid/acp#memberAccessControl",
    "http://www.w3.org/ns/solid/acp#apply",
    "http://www.w3.org/ns/solid/acp#allOf",
    "http://www.w3.org/ns/solid/acp#anyOf",
    "http://www.w3.org/ns/solid/acp#noneOf",
    "http://www.w3.org/ns/solid/acp#agent",
    "http://www.w3.org/ns/solid/acp#client",
    "http://www.w3.org/ns/solid/acp#issuer",
    "http://www.w3.org/ns/solid/acp#vc",
    "http://www.w3.org/ns/solid/acp#allow",
    "http://www.w3.org/ns/solid/acp#deny",
];

/// One reasoning-input document and the facts the reasoner reads from it.
pub(crate) struct InputDoc {
    pub iri: String,
    /// A control document of the system (as opposed to a WAC group document).
    pub control: bool,
    pub facts: Vec<[Term; 3]>,
}

/// The reasoning-input documents of `graph` for `system`, each with its attributed facts.
/// A superset of the loader's selection: WAC group documents named from either system's
/// control documents are included.
pub(crate) fn input_docs(graph: &Graph, system: System) -> Vec<InputDoc> {
    let suffix = if system == System::Wac {
        ACL_SUFFIX
    } else {
        ACR_SUFFIX
    };
    let groups = if system == System::Wac {
        referenced_group_docs(graph)
    } else {
        FxHashSet::default()
    };
    let (owns, in_doc) = (own_predicate(system), in_doc_predicate());
    let mut out = Vec::new();
    for (name, sub) in &graph.named {
        let Some(iri) = graph_iri(name) else { continue };
        if iri.starts_with(RESERVED_PREFIX) {
            continue;
        }
        let resource = iri.strip_suffix(suffix).filter(|r| !is_authority_less(r));
        let control = resource.is_some();
        if !control && !groups.contains(iri) {
            continue;
        }
        let mut facts: Vec<[Term; 3]> = graph_triples(sub)
            .iter()
            .filter_map(|t| input_fact(t, iri))
            .collect();
        if let Some(r) = resource {
            let doc = Term::NamedNode(NamedNode::new_unchecked(iri));
            let subjects: FxHashSet<Term> = facts.iter().map(|f| f[0].clone()).collect();
            facts.extend(
                subjects
                    .into_iter()
                    .map(|s| [s, in_doc.clone(), doc.clone()]),
            );
            facts.push([
                Term::NamedNode(NamedNode::new_unchecked(r)),
                owns.clone(),
                doc,
            ]);
        }
        out.push(InputDoc {
            iri: iri.to_owned(),
            control,
            facts,
        });
    }
    out
}

/// The dependency graph between the reasoning-input documents of one access-control system.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlDeps {
    /// Control document → every other input document its grants can depend on.
    pub forward: FxHashMap<String, FxHashSet<String>>,
    /// Input document → every control document that depends on it.
    pub reverse: FxHashMap<String, FxHashSet<String>>,
}

impl ControlDeps {
    /// The dependencies among `graph`'s reasoning-input documents for `system`.
    pub(crate) fn build(graph: &Graph, system: System) -> Self {
        let inputs = input_docs(graph, system);
        let (owns, in_doc) = (own_predicate(system), in_doc_predicate());
        // Each input document's subjects, linked nodes and named group documents.
        let mut docs: FxHashMap<&str, Doc> = FxHashMap::default();
        let mut controls: Vec<&str> = Vec::new();
        for input in &inputs {
            if input.control {
                controls.push(&input.iri);
            }
            let doc = docs.entry(&input.iri).or_default();
            for [s, p, o] in &input.facts {
                if let Term::NamedNode(s) = s {
                    doc.subjects.insert(s.as_str().to_owned());
                }
                let (Term::NamedNode(pn), Term::NamedNode(on)) = (p, o) else {
                    continue;
                };
                if pn.as_str() == ACL_AGENT_GROUP {
                    doc.groups.insert(group_doc_of(on.as_str()).to_owned());
                }
                if LINKAGE.contains(&pn.as_str()) || *p == owns || *p == in_doc {
                    doc.links.insert(on.as_str().to_owned());
                }
            }
        }
        // Subject → the input documents describing it.
        let mut described_in: FxHashMap<&str, Vec<&str>> = FxHashMap::default();
        for (iri, doc) in &docs {
            for s in &doc.subjects {
                described_in.entry(s.as_str()).or_default().push(iri);
            }
        }
        let neighbours = |iri: &str| -> FxHashSet<&str> {
            let doc = &docs[iri];
            let mut out: FxHashSet<&str> = FxHashSet::default();
            for x in doc.links.iter().chain(&doc.subjects) {
                out.extend(described_in.get(x.as_str()).into_iter().flatten().copied());
            }
            out.extend(
                doc.groups
                    .iter()
                    .filter_map(|g| docs.get_key_value(g.as_str()))
                    .map(|(k, _)| *k),
            );
            out
        };

        let mut deps = ControlDeps::default();
        for control in controls {
            let mut seen: FxHashSet<&str> = FxHashSet::from_iter([control]);
            let mut todo = vec![control];
            while let Some(d) = todo.pop() {
                for n in neighbours(d) {
                    if seen.insert(n) {
                        todo.push(n);
                    }
                }
            }
            seen.remove(control);
            for d in &seen {
                deps.reverse
                    .entry((*d).to_owned())
                    .or_default()
                    .insert(control.to_owned());
            }
            deps.forward.insert(
                control.to_owned(),
                seen.into_iter().map(str::to_owned).collect(),
            );
        }
        deps
    }
}

/// One input document's subjects, linked nodes and named group documents.
#[derive(Default)]
struct Doc {
    subjects: FxHashSet<String>,
    links: FxHashSet<String>,
    groups: FxHashSet<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(nq: &str, system: System) -> ControlDeps {
        ControlDeps::build(&Graph::load_dataset(nq, "nquads").expect("loads"), system)
    }

    fn set(xs: &[&str]) -> FxHashSet<String> {
        xs.iter().map(|x| (*x).to_owned()).collect()
    }

    const ACL: &str = "http://www.w3.org/ns/auth/acl#";
    const ACP: &str = "http://www.w3.org/ns/solid/acp#";

    #[test]
    fn a_pod_that_references_nothing_depends_on_nothing() {
        let d = deps(&crate::wac_fixture(), System::Wac);
        assert!(!d.forward.is_empty());
        for (acl, on) in &d.forward {
            assert!(on.iter().all(|x| !x.ends_with(".acl")), "{acl} -> {on:?}");
        }
    }

    #[test]
    fn a_group_document_is_a_dependency_and_its_readers_are_reverse_edges() {
        let nq = format!(
            "<https://a.ex/.acl#r> <{ACL}agentGroup> <https://g.ex/team#t> <https://a.ex/.acl> .\n\
             <https://b.ex/.acl#r> <{ACL}agentGroup> <https://g.ex/team#t> <https://b.ex/.acl> .\n\
             <https://c.ex/.acl#r> <{ACL}agent> <https://c.ex/me> <https://c.ex/.acl> .\n\
             <https://g.ex/team#t> <http://www.w3.org/2006/vcard/ns#hasMember> <https://m.ex/me> <https://g.ex/team> .\n"
        );
        let d = deps(&nq, System::Wac);
        assert_eq!(d.forward["https://a.ex/.acl"], set(&["https://g.ex/team"]));
        assert_eq!(d.forward["https://c.ex/.acl"], set(&[]));
        assert_eq!(
            d.reverse["https://g.ex/team"],
            set(&["https://a.ex/.acl", "https://b.ex/.acl"])
        );
    }

    #[test]
    fn acp_indirection_follows_policies_and_matchers_across_documents() {
        let nq = format!(
            "<https://b.ex/.acr> <{ACP}accessControl> <https://b.ex/.acr#c> <https://b.ex/.acr> .\n\
             <https://b.ex/.acr#c> <{ACP}apply> <https://a.ex/.acr#pol> <https://b.ex/.acr> .\n\
             <https://a.ex/.acr#pol> <{ACP}allOf> <https://m.ex/.acr#m> <https://a.ex/.acr> .\n\
             <https://m.ex/.acr#m> <{ACP}agent> <https://alice.ex/me> <https://m.ex/.acr> .\n\
             <https://z.ex/.acr#unrelated> <{ACP}agent> <https://alice.ex/me> <https://z.ex/.acr> .\n"
        );
        let d = deps(&nq, System::Acp);
        assert_eq!(
            d.forward["https://b.ex/.acr"],
            set(&["https://a.ex/.acr", "https://m.ex/.acr"])
        );
        assert_eq!(d.forward["https://a.ex/.acr"], set(&["https://m.ex/.acr"]));
        assert_eq!(
            d.forward["https://z.ex/.acr"],
            set(&[]),
            "a shared object links nothing"
        );
        assert_eq!(
            d.reverse["https://m.ex/.acr"],
            set(&["https://a.ex/.acr", "https://b.ex/.acr"])
        );
    }

    #[test]
    fn documents_describing_one_subject_depend_on_each_other() {
        let nq = format!(
            "<https://o.ex/.acl#a1> <{ACL}accessTo> <https://o.ex/n1> <https://o.ex/.acl> .\n\
             <https://o.ex/.acl#a1> <{ACL}agent> <https://mallory.ex/me> <https://f.ex/.acl> .\n"
        );
        let d = deps(&nq, System::Wac);
        assert_eq!(d.forward["https://o.ex/.acl"], set(&["https://f.ex/.acl"]));
        assert_eq!(d.forward["https://f.ex/.acl"], set(&["https://o.ex/.acl"]));
    }

    #[test]
    fn the_other_systems_documents_are_not_inputs() {
        let nq = format!(
            "<https://a.ex/.acl#r> <{ACL}agent> <https://p.ex/.acr#pol> <https://a.ex/.acl> .\n\
             <https://p.ex/.acr#pol> <{ACP}allow> <{ACL}Read> <https://p.ex/.acr> .\n"
        );
        let wac = deps(&nq, System::Wac);
        assert_eq!(wac.forward["https://a.ex/.acl"], set(&[]));
        assert!(!wac.forward.contains_key("https://p.ex/.acr"));
    }

    #[test]
    fn a_document_describing_a_control_documents_own_iri_is_a_dependency() {
        // The loader's `<b/> solidx:ownAcr <b/.acr>` makes the control document's IRI a
        // join key even when the document never mentions itself.
        let nq = format!(
            "<https://b.ex/.acr#c> <{ACP}apply> <https://b.ex/.acr#p> <https://b.ex/.acr> .\n\
             <https://b.ex/.acr> <{ACP}accessControl> <https://f.ex/.acr#c> <https://f.ex/.acr> .\n"
        );
        let d = deps(&nq, System::Acp);
        assert_eq!(d.forward["https://b.ex/.acr"], set(&["https://f.ex/.acr"]));
        assert_eq!(d.reverse["https://f.ex/.acr"], set(&["https://b.ex/.acr"]));
    }

    #[test]
    fn a_blank_node_meets_the_iri_it_is_skolemized_to() {
        let skolem = match crate::loader::skolemize(
            &Term::BlankNode(oxrdf::BlankNode::new_unchecked("p")),
            "https://b.ex/.acr",
        ) {
            Term::NamedNode(n) => n.as_str().to_owned(),
            other => panic!("expected an IRI, got {other}"),
        };
        let nq = format!(
            "<https://b.ex/.acr#c> <{ACP}apply> _:p <https://b.ex/.acr> .\n\
             <{skolem}> <{ACP}allOf> <https://m.ex/.acr#m> <https://f.ex/.acr> .\n"
        );
        let d = deps(&nq, System::Acp);
        assert_eq!(d.forward["https://b.ex/.acr"], set(&["https://f.ex/.acr"]));
        assert_eq!(
            d.forward["https://f.ex/.acr"],
            set(&[]),
            "f links nothing of b"
        );
    }

    /// Every fact the reasoner reads is attributed to an input document, except the
    /// loader's per-IRI markers, which no document states.
    #[test]
    fn input_docs_attribute_every_assembled_fact() {
        let markers = [
            format!("{}isResource", crate::SOLIDX_NS),
            format!("{}isWebId", crate::SOLIDX_NS),
        ];
        let blanks = format!(
            "_:a <{ACL}agentGroup> <https://g.ex/team#t> <https://a.ex/.acl> .\n\
             _:a <{ACL}accessTo> <https://a.ex/> <https://a.ex/.acl> .\n\
             _:a <{ACL}mode> <{ACL}Read> <https://a.ex/.acl> .\n\
             <https://g.ex/team#t> <http://www.w3.org/2006/vcard/ns#hasMember> _:m <https://g.ex/team> .\n\
             <https://b.ex/.acr> <{ACP}accessControl> _:c <https://b.ex/.acr> .\n\
             _:c <{ACP}apply> _:p <https://b.ex/.acr> .\n\
             _:p <{ACP}allOf> _:m <https://b.ex/.acr> .\n\
             _:m <{ACP}agent> <https://alice.ex/me> <https://b.ex/.acr> .\n\
             <https://b.ex/r> <{ACP}agent> <https://bob.ex/me> <https://b.ex/r> .\n"
        );
        let mut checked = 0;
        for nq in [crate::wac_fixture(), crate::acp_fixture(), blanks] {
            let graph = Graph::load_dataset(&nq, "nquads").expect("loads");
            for system in [System::Wac, System::Acp] {
                let attributed: FxHashSet<[Term; 3]> = input_docs(&graph, system)
                    .into_iter()
                    .flat_map(|d| d.facts)
                    .collect();
                let assembled = crate::loader::assemble_facts(
                    &graph,
                    system,
                    &crate::AccessProvenance::default(),
                    &crate::VerifiedCredentials::default(),
                )
                .expect("assembles");
                for fact in assembled {
                    if matches!(&fact[1], Term::NamedNode(p) if markers.iter().any(|m| m == p.as_str()))
                    {
                        continue;
                    }
                    checked += 1;
                    assert!(
                        attributed.contains(&fact),
                        "{system:?}: unattributed {fact:?}"
                    );
                }
            }
        }
        assert!(checked > 50, "checked {checked} facts");
    }

    /// Every document predicate a rule body binds to a variable object is in [`LINKAGE`],
    /// at any depth (negated `log:notIncludes` formulas included). A subject-position
    /// variable needs no entry: every attributed subject is indexed. Predicates in the
    /// reasoner's own namespaces are derived or built in, except the loader's structural
    /// facts, which [`ControlDeps::build`] links itself. A variable predicate is accepted
    /// only where the same rule binds it from a derived `solidx:` fact, which no document
    /// can state (the loader drops `solidx:` predicates).
    #[test]
    fn linkage_covers_every_rule_body_predicate() {
        use sparq_reason::n3::Term as N3;
        const OWN: [&str; 4] = [
            "https://sparq.dev/ns/solidx#",
            "https://sparq.dev/ns/auth#",
            "http://www.w3.org/2000/10/swap/log#",
            "http://www.w3.org/2000/10/swap/string#",
        ];
        let solidx = crate::SOLIDX_NS;
        let structural = [
            format!("{solidx}ownAcl"),
            format!("{solidx}ownAcr"),
            format!("{solidx}inDoc"),
        ];
        fn atoms<'a>(ts: &'a [[N3; 3]], out: &mut Vec<&'a [N3; 3]>) {
            for t in ts {
                out.push(t);
                for x in t {
                    match x {
                        N3::Formula(inner) => atoms(inner, out),
                        N3::Triple(inner) => atoms(std::slice::from_ref(&**inner), out),
                        _ => {}
                    }
                }
            }
        }
        let rules = [
            include_str!("../rules/common.n3"),
            include_str!("../rules/wac.n3"),
            include_str!("../rules/acp-a.n3"),
            include_str!("../rules/acp-b.n3"),
            include_str!("../rules/acp-c.n3"),
        ];
        let (mut found, mut structural_found) = (0, 0);
        for text in rules {
            let parsed = sparq_reason::n3::parser::parse(text).expect("rules parse");
            assert!(
                parsed.backward_rules.is_empty(),
                "backward rules are not indexed"
            );
            for rule in &parsed.rules {
                let mut body = Vec::new();
                atoms(&rule.premise, &mut body);
                for [_, p, o] in body.iter().copied() {
                    match p {
                        N3::Var(v) => assert!(
                            body.iter().any(|[_, q, b]| *b == N3::Var(v.clone())
                                && matches!(q, N3::Iri(q) if q.starts_with(solidx))),
                            "rule body has a document-reachable variable predicate ?{v}"
                        ),
                        N3::Iri(p) if matches!(o, N3::Var(_)) => {
                            if structural.contains(p) {
                                structural_found += 1;
                            } else if !OWN.iter().any(|ns| p.starts_with(ns)) {
                                found += 1;
                                assert!(
                                    LINKAGE.contains(&p.as_str()),
                                    "rule body binds the object of <{p}>"
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        assert!(found > 10, "the rule scan matched {found} predicates");
        assert!(
            structural_found > 0,
            "the rule scan matched no structural predicate"
        );
    }
}
