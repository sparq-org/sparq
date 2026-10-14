//! Which reasoning-input documents each access-control document's grants can depend on, and
//! the reverse (issue #5312; `research/solid-pod-scoped-materializer-design.md` §5.2).
//!
//! The materializer reasons over one fact pool: every control document of the system plus,
//! for WAC, every group document an `acl:agentGroup` names. Blank nodes are skolemized per
//! document, so two documents can only meet through a shared IRI. A document's grants can
//! therefore change when another document
//!
//! - defines a node this one links to through a [`LINKAGE`] predicate (an ACP control naming
//!   a policy or matcher defined elsewhere, a WAC group whose members a group document
//!   lists), or
//! - states facts about a subject this one also describes (two documents sharing an
//!   authorization or policy IRI, case (C) of the design record).
//!
//! [`ControlDeps::build`] follows both edge kinds to a fixpoint. It only derives state; no
//! materialization reads it yet.

use crate::loader::{
    graph_iri, graph_triples, group_doc_of, referenced_group_docs, System, ACL_AGENT_GROUP,
    ACL_SUFFIX, ACR_SUFFIX, RESERVED_PREFIX,
};
use oxrdf::Term;
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::Graph;

/// The document predicates whose object a rule body binds to a variable, so the object can
/// join a fact from another document. `linkage_covers_every_rule_body_predicate` re-reads
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
        // Each input document's IRI-subject set and linked IRIs.
        let mut docs: FxHashMap<&str, Doc> = FxHashMap::default();
        let mut controls: Vec<&str> = Vec::new();
        for (name, sub) in &graph.named {
            let Some(iri) = graph_iri(name) else { continue };
            if iri.starts_with(RESERVED_PREFIX) {
                continue;
            }
            let control = iri.ends_with(suffix);
            if !control && !groups.contains(iri) {
                continue;
            }
            if control {
                controls.push(iri);
            }
            let doc = docs.entry(iri).or_default();
            for t in graph_triples(sub) {
                if let Term::NamedNode(s) = &t[0] {
                    doc.subjects.insert(s.as_str().to_owned());
                }
                let (Term::NamedNode(p), Term::NamedNode(o)) = (&t[1], &t[2]) else {
                    continue;
                };
                if p.as_str() == ACL_AGENT_GROUP {
                    doc.groups.insert(group_doc_of(o.as_str()).to_owned());
                }
                if LINKAGE.contains(&p.as_str()) {
                    doc.links.insert(o.as_str().to_owned());
                }
            }
        }
        // Subject IRI → the input documents describing it.
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

/// One input document's IRI subjects, linked objects and named group documents.
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

    /// Every document predicate a rule body binds to a variable object is in [`LINKAGE`].
    /// A predicate in the reasoner's own namespaces (`solidx:`, `auth:`, the `log:`/`string:`
    /// builtins) is derived or built in, never read from a document.
    #[test]
    fn linkage_covers_every_rule_body_predicate() {
        const OWN: [&str; 4] = [
            "https://sparq.dev/ns/solidx#",
            "https://sparq.dev/ns/auth#",
            "http://www.w3.org/2000/10/swap/log#",
            "http://www.w3.org/2000/10/swap/string#",
        ];
        let rules = [
            include_str!("../rules/common.n3"),
            include_str!("../rules/wac.n3"),
            include_str!("../rules/acp-a.n3"),
            include_str!("../rules/acp-b.n3"),
            include_str!("../rules/acp-c.n3"),
        ];
        let mut found = 0;
        for text in rules {
            let mut prefixes: FxHashMap<&str, &str> = FxHashMap::default();
            let mut tokens: Vec<&str> = Vec::new();
            for line in text.lines() {
                let line = line.split(" #").next().unwrap_or(line);
                let line = if line.trim_start().starts_with('#') {
                    ""
                } else {
                    line
                };
                let words: Vec<&str> = line.split_whitespace().collect();
                if words.first() == Some(&"@prefix") {
                    let iri = words[2].trim_start_matches('<').trim_end_matches('>');
                    prefixes.insert(words[1].trim_end_matches(':'), iri);
                    continue;
                }
                tokens.extend(words);
            }
            for w in tokens.windows(3) {
                let (p, o) = (w[1], w[2]);
                if !o.starts_with('?') || p.starts_with('?') {
                    continue;
                }
                let Some((prefix, local)) = p.split_once(':') else {
                    continue;
                };
                let Some(ns) = prefixes.get(prefix) else {
                    continue;
                };
                if OWN.contains(ns) {
                    continue;
                }
                let iri = format!("{ns}{local}");
                found += 1;
                assert!(
                    LINKAGE.contains(&iri.as_str()),
                    "rule body binds the object of <{iri}>"
                );
            }
        }
        assert!(found > 10, "the rule scan matched {found} predicates");
    }
}
