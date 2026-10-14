//! How the trusted creator/owner provenance follows a change to the stored graphs.
//!
//! Every change the store makes to which graphs exist (an update's effects, or a caller's
//! direct edit such as the MCP server's) is replayed, in order, against one rule for which
//! resources exist ([`ResourceGraphs`], the loader's own enumeration). A resource whose graph
//! is removed, or that stops being a resource, at ANY point of the change loses its creator
//! and owner; a resource the change brings into being (one that was not a resource just
//! before, or whose life ended earlier in the same change) gets the acting agent as its
//! creator. The result is staged with the change and installed only once the change commits.

use crate::loader::{validate_principal_iri, ResourceGraphs};
use crate::AccessProvenance;
use oxrdf::Term;
use rustc_hash::FxHashSet;
use spargebra::algebra::GraphTarget;
use sparq_core::Graph;

/// One change to which graphs are stored, in the order the change makes them.
#[derive(Clone, Copy, Debug)]
pub(crate) enum GraphEvent<'a> {
    /// A graph of this name is stored (a no-op when one already is).
    Added(&'a str),
    /// The graph of this name is removed (a no-op when none is stored).
    Removed(&'a str),
    /// Every named graph is removed.
    RemovedAll,
}

/// The events an update's `effects` make against `graph`, the store before the update: a
/// graph created (by `CREATE`, or by an insert into a graph not yet stored) or dropped. A
/// `CLEAR` or a delete leaves the graph stored.
pub(crate) fn update_events<'e>(
    graph: &Graph,
    effects: &'e [sparq_engine::UpdateEffect],
) -> Vec<GraphEvent<'e>> {
    use sparq_engine::UpdateEffect;
    let mut events = Vec::new();
    for effect in effects {
        match effect {
            UpdateEffect::Delta {
                slot: Some(Term::NamedNode(n)),
                inserts,
                ..
            } if !inserts.is_empty() => {
                events.push(GraphEvent::Added(n.as_str()));
            }
            UpdateEffect::Create(Term::NamedNode(n)) => events.push(GraphEvent::Added(n.as_str())),
            UpdateEffect::Drop(GraphTarget::NamedNode(n)) => {
                events.push(GraphEvent::Removed(n.as_str()));
            }
            UpdateEffect::Drop(GraphTarget::NamedGraphs | GraphTarget::AllGraphs) => {
                events.push(GraphEvent::RemovedAll);
            }
            _ => {}
        }
    }
    // An update that only adds to graphs already stored changes no resource; most do.
    let changes = |e: &GraphEvent| match e {
        GraphEvent::Added(n) => graph
            .named_graph(&Term::NamedNode(oxrdf::NamedNode::new_unchecked(*n)))
            .is_none(),
        _ => true,
    };
    if events.iter().any(changes) {
        events
    } else {
        Vec::new()
    }
}

/// The events a caller's direct edit made, from the graph names stored before it and the
/// store after it: every graph it removed, then every graph it added (a caller's edit
/// touches each graph once).
pub(crate) fn edit_events<'a>(
    before: &'a FxHashSet<String>,
    after: &'a Graph,
) -> Vec<GraphEvent<'a>> {
    let now: FxHashSet<&str> = after
        .named
        .iter()
        .filter_map(|(n, _)| match n {
            Term::NamedNode(n) => Some(n.as_str()),
            _ => None,
        })
        .collect();
    let mut events: Vec<GraphEvent> = before
        .iter()
        .filter(|n| !now.contains(n.as_str()))
        .map(|n| GraphEvent::Removed(n))
        .collect();
    events.extend(
        now.iter()
            .filter(|n| !before.contains(**n))
            .map(|n| GraphEvent::Added(n)),
    );
    events
}

/// `provenance` after `events` change `before` (the store's graphs before them), with
/// `creator` (the acting agent, when the view would take it as a principal) recorded as the
/// creator of every resource the change brings into being.
pub(crate) fn provenance_after(
    before: &Graph,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
) -> AccessProvenance {
    if events.is_empty() {
        return provenance.clone();
    }
    replay(ResourceGraphs::of(before), events, provenance, creator)
}

/// [`provenance_after`] from the names of the graphs stored before the events.
pub(crate) fn provenance_after_names(
    before: &FxHashSet<String>,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
) -> AccessProvenance {
    if events.is_empty() {
        return provenance.clone();
    }
    replay(
        ResourceGraphs::from_names(before.iter().map(String::as_str)),
        events,
        provenance,
        creator,
    )
}

fn replay(
    mut graphs: ResourceGraphs,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
) -> AccessProvenance {
    let mut ended: FxHashSet<String> = FxHashSet::default();
    let mut began: FxHashSet<String> = FxHashSet::default();
    for event in events {
        match *event {
            GraphEvent::Added(name) => {
                let new_life = !graphs.holds(name) || ended.contains(name);
                graphs.add(name, |r| {
                    began.insert(r.to_owned());
                });
                if new_life && graphs.holds(name) {
                    began.insert(name.to_owned());
                }
            }
            GraphEvent::Removed(name) => {
                // A removed graph's life ends even when its name stays a container.
                if graphs.holds(name) {
                    ended.insert(name.to_owned());
                }
                graphs.remove(name, |r| {
                    ended.insert(r.to_owned());
                });
            }
            GraphEvent::RemovedAll => graphs.clear(|r| {
                ended.insert(r.to_owned());
            }),
        }
    }
    let mut kept = provenance.clone();
    for r in &ended {
        kept.forget(r);
    }
    if let Some(creator) = creator.filter(|c| validate_principal_iri(c).is_ok()) {
        for r in began.iter().filter(|r| graphs.holds(r)) {
            kept.set_creator(r.as_str(), creator);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::{provenance_after, GraphEvent};
    use crate::AccessProvenance;
    use sparq_core::Graph;

    const ALICE: &str = "https://alice.ex/card#me";
    const BOB: &str = "https://bob.ex/card#me";

    fn store(names: &[&str]) -> Graph {
        let nq: String = names
            .iter()
            .map(|n| format!("<{n}#it> <https://ex.dev/ns#p> \"x\" <{n}> .\n"))
            .collect();
        Graph::load_dataset(&nq, "nquads").expect("loads")
    }

    fn created_by(names: &[&str], who: &str) -> AccessProvenance {
        let mut p = AccessProvenance::new();
        for n in names {
            p.set_creator(*n, who);
        }
        p
    }

    #[test]
    fn a_container_that_goes_and_comes_back_in_one_change_is_new() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/keep"]);
        let prov = created_by(&["https://pod.ex/a/", "https://pod.ex/a/x"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x"),
            GraphEvent::Added("https://pod.ex/a/y"),
        ];
        let after = provenance_after(&before, &events, &prov, Some(BOB));
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            Some(BOB),
            "a new container"
        );
        assert_eq!(after.creator("https://pod.ex/a/x"), None, "gone");
        assert_eq!(after.creator("https://pod.ex/a/y"), Some(BOB));
        assert_eq!(
            after.creator("https://pod.ex/"),
            None,
            "the root never went"
        );
    }

    #[test]
    fn a_container_kept_by_another_graph_keeps_its_creator() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/a/z"]);
        let prov = created_by(&["https://pod.ex/a/"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x"),
            GraphEvent::Added("https://pod.ex/a/"),
        ];
        let after = provenance_after(&before, &events, &prov, Some(BOB));
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            Some(ALICE),
            "never stopped existing"
        );
    }

    #[test]
    fn a_removed_graph_loses_its_provenance_even_when_its_name_stays_a_container() {
        let before = store(&["https://pod.ex/a/", "https://pod.ex/a/x"]);
        let prov = created_by(&["https://pod.ex/a/"], ALICE);
        let after = provenance_after(
            &before,
            &[GraphEvent::Removed("https://pod.ex/a/")],
            &prov,
            Some(BOB),
        );
        assert_eq!(after.creator("https://pod.ex/a/"), None);
        let back = [
            GraphEvent::Removed("https://pod.ex/a/"),
            GraphEvent::Added("https://pod.ex/a/"),
        ];
        let after = provenance_after(&before, &back, &prov, Some(BOB));
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            Some(BOB),
            "created again in the change"
        );
    }

    #[test]
    fn every_container_an_odd_path_brings_is_new() {
        let before = store(&["https://pod.ex/x"]);
        let name = "https://pod.ex/a//b";
        let after = provenance_after(
            &before,
            &[GraphEvent::Added(name)],
            &AccessProvenance::new(),
            Some(BOB),
        );
        for r in [name, "https://pod.ex/a//", "https://pod.ex/a/"] {
            assert_eq!(after.creator(r), Some(BOB), "{r}");
        }
        assert_eq!(
            after.creator("https://pod.ex/"),
            None,
            "the root already existed"
        );
    }

    #[test]
    fn dropping_everything_forgets_everything_and_an_unusable_creator_records_nothing() {
        let before = store(&["https://pod.ex/a/x"]);
        let prov = created_by(
            &["https://pod.ex/a/", "https://pod.ex/a/x", "https://pod.ex/"],
            ALICE,
        );
        let events = [
            GraphEvent::RemovedAll,
            GraphEvent::Added("https://pod.ex/a/x"),
        ];
        let after = provenance_after(&before, &events, &prov, Some("urn:sparq:x"));
        assert!(after.is_empty(), "forgotten, and no creator: {after:?}");
    }

    #[test]
    fn the_resource_rule_matches_the_loaders_enumeration() {
        let names = [
            "https://pod.ex/a/b/c",
            "https://pod.ex/a//d",
            "https://pod.ex/e/",
            "https://pod.ex/e/.acr",
            "https://pod.ex/f/.acl",
            "urn:sparq:auth",
            "https://",
            "https://other.ex/",
            "https://pod.ex/g?x=/y",
        ];
        let graph = store(&names);
        let enumerated = crate::loader::resources(&graph);
        let graphs = crate::loader::ResourceGraphs::of(&graph);
        assert_eq!(graphs.all(), enumerated);
        let mut probes: Vec<String> = enumerated.iter().cloned().collect();
        probes.extend(
            [
                "https://pod.ex/f/",
                "https://pod.ex/a/b/c/",
                "https://pod.ex/g?x=/",
            ]
            .map(String::from),
        );
        for p in probes {
            assert_eq!(graphs.holds(&p), enumerated.contains(&p), "{p}");
        }
    }
}
