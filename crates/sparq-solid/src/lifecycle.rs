//! How the trusted creator/owner provenance follows a change to the stored graphs.
//!
//! Every change the store makes to which graphs exist (an update's effects, or a caller's
//! direct edit such as the MCP server's) is replayed, in order, against one rule for which
//! resources exist ([`ResourceGraphs`], the loader's own enumeration), and only its net
//! effect counts. A resource that existed before the change and does not after it (a
//! document whose graph is removed, a container left with no graph below it) loses its
//! creator and owner, unless its own access-control document (`<r>.acl`/`<r>.acr`) is still
//! stored: they are kept with that document and come back with the resource. One that
//! exists after the change and did not before gets the acting agent as its creator, when
//! there is one (a SPARQL update records none) and its own access-control document was not
//! stored before the change. A resource that exists
//! before and after keeps both, whatever happens in between. The result is staged with the
//! change and installed only once the change commits.

use crate::loader::{validate_principal_iri, ResourceGraphs, ACL_SUFFIX, ACR_SUFFIX};
use crate::AccessProvenance;
use oxrdf::Term;
use rustc_hash::{FxHashMap, FxHashSet};
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
    let stored = |name: &str| {
        graph
            .named
            .iter()
            .any(|(n, _)| matches!(n, Term::NamedNode(n) if n.as_str() == name))
    };
    let changes = |e: &GraphEvent| match e {
        GraphEvent::Added(n) => !stored(n),
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
/// creator of every resource that exists after them and did not before, unless its own
/// access-control document was stored before them (see the module docs); `None` when the
/// events change no resource's life (most updates), so nothing is copied.
pub(crate) fn provenance_after(
    before: &Graph,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
) -> Option<AccessProvenance> {
    if events.is_empty() {
        return None;
    }
    let stored = |name: &str| {
        before
            .named
            .iter()
            .any(|(n, _)| matches!(n, Term::NamedNode(n) if n.as_str() == name))
    };
    replay(
        ResourceGraphs::of(before),
        events,
        provenance,
        creator,
        stored,
    )
}

/// [`provenance_after`] from the names of the graphs stored before the events.
pub(crate) fn provenance_after_names(
    before: &FxHashSet<String>,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
) -> Option<AccessProvenance> {
    if events.is_empty() {
        return None;
    }
    let stored = |name: &str| before.contains(name);
    replay(
        ResourceGraphs::from_names(before.iter().map(String::as_str)),
        events,
        provenance,
        creator,
        stored,
    )
}

fn replay(
    mut graphs: ResourceGraphs,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
    stored_before: impl Fn(&str) -> bool,
) -> Option<AccessProvenance> {
    // Each resource the change touches, with whether it was one before the change (the first
    // time the change touches it says which). Only the net effect counts: a resource that
    // exists before and after keeps its creator and owner, whatever happens in between.
    let mut touched: FxHashMap<String, bool> = FxHashMap::default();
    // The access-control documents the change adds or removes, and whether it removes every
    // graph, to tell which are stored after it.
    let mut control: FxHashMap<&str, bool> = FxHashMap::default();
    let mut cleared = false;
    let is_control = |n: &str| n.ends_with(ACL_SUFFIX) || n.ends_with(ACR_SUFFIX);
    for event in events {
        match *event {
            GraphEvent::Added(name) => {
                if is_control(name) {
                    control.insert(name, true);
                }
                graphs.add(name, |r| {
                    touched.entry(r.to_owned()).or_insert(false);
                });
            }
            GraphEvent::Removed(name) => {
                if is_control(name) {
                    control.insert(name, false);
                }
                graphs.remove(name, |r| {
                    touched.entry(r.to_owned()).or_insert(true);
                });
            }
            GraphEvent::RemovedAll => {
                control.clear();
                cleared = true;
                graphs.clear(|r| {
                    touched.entry(r.to_owned()).or_insert(true);
                });
            }
        }
    }
    let stored_after = |name: &str| {
        control
            .get(name)
            .copied()
            .unwrap_or_else(|| !cleared && stored_before(name))
    };
    let own_control = |r: &str, stored: &dyn Fn(&str) -> bool| {
        stored(&format!("{r}{ACL_SUFFIX}")) || stored(&format!("{r}{ACR_SUFFIX}"))
    };
    let creator = creator.filter(|c| validate_principal_iri(c).is_ok());
    let mut ended = Vec::new();
    let mut began = Vec::new();
    for (r, was) in &touched {
        match (*was, graphs.holds(r)) {
            // A resource whose own access-control document is still stored keeps its creator
            // and owner: they come back with it, under that document's rules.
            (true, false) if provenance.knows(r) && !own_control(r, &stored_after) => {
                ended.push(r);
            }
            // So whoever brings such a resource back is not its creator.
            (false, true) if creator.is_some() && !own_control(r, &stored_before) => {
                began.push(r);
            }
            _ => {}
        }
    }
    if ended.is_empty() && began.is_empty() {
        return None;
    }
    let mut kept = provenance.clone();
    for r in ended {
        kept.forget(r);
    }
    if let Some(creator) = creator {
        for r in began {
            kept.forget(r);
            kept.set_creator(r.as_str(), creator);
        }
    }
    Some(kept)
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
    fn a_container_that_goes_and_comes_back_in_one_change_keeps_its_creator() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/keep"]);
        let prov = created_by(&["https://pod.ex/a/", "https://pod.ex/a/x"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x"),
            GraphEvent::Added("https://pod.ex/a/y"),
        ];
        let after = provenance_after(&before, &events, &prov, Some(BOB)).expect("a change");
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            Some(ALICE),
            "/a/ exists before and after"
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
    fn a_document_removed_and_stored_again_in_one_change_keeps_its_creator() {
        let before = store(&["https://pod.ex/a/x"]);
        let prov = created_by(&["https://pod.ex/a/x"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x"),
            GraphEvent::Added("https://pod.ex/a/x"),
        ];
        assert!(provenance_after(&before, &events, &prov, Some(BOB)).is_none());
    }

    #[test]
    fn a_resource_brought_back_under_its_surviving_access_control_document_gets_no_creator() {
        let before = store(&["https://pod.ex/a/.acr", "https://pod.ex/b/x.acl"]);
        let events = [
            GraphEvent::Added("https://pod.ex/a/y"),
            GraphEvent::Added("https://pod.ex/b/x"),
        ];
        let after = provenance_after(&before, &events, &AccessProvenance::new(), Some(BOB))
            .expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/y"), Some(BOB), "new");
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            None,
            "/a/.acr outlived /a/"
        );
        assert_eq!(
            after.creator("https://pod.ex/b/x"),
            None,
            "x.acl outlived x"
        );
        assert_eq!(after.creator("https://pod.ex/b/"), Some(BOB), "new");
    }

    #[test]
    fn a_resource_whose_access_control_document_survives_keeps_its_creator() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/a/.acr"]);
        let prov = created_by(&["https://pod.ex/a/", "https://pod.ex/a/x"], ALICE);
        let gone = [GraphEvent::Removed("https://pod.ex/a/x")];
        let after = provenance_after(&before, &gone, &prov, Some(BOB)).expect("a change");
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            Some(ALICE),
            "kept with /a/.acr"
        );
        assert_eq!(
            after.creator("https://pod.ex/a/x"),
            None,
            "x had no document of its own"
        );
        let both = [
            GraphEvent::Removed("https://pod.ex/a/x"),
            GraphEvent::Removed("https://pod.ex/a/.acr"),
        ];
        let after = provenance_after(&before, &both, &prov, Some(BOB)).expect("a change");
        assert_eq!(
            after.creator("https://pod.ex/a/"),
            None,
            "its document went too"
        );
        let all = [GraphEvent::RemovedAll];
        let after = provenance_after(&before, &all, &prov, Some(BOB)).expect("a change");
        assert!(after.is_empty(), "everything went: {after:?}");
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
        assert!(after.is_none(), "/a/ never stopped existing: {after:?}");
    }

    #[test]
    fn a_container_with_members_keeps_its_provenance_whatever_happens_to_its_own_graph() {
        let before = store(&["https://pod.ex/a/", "https://pod.ex/a/x"]);
        let prov = created_by(&["https://pod.ex/a/"], ALICE);
        for events in [
            &[GraphEvent::Removed("https://pod.ex/a/")][..],
            &[
                GraphEvent::Removed("https://pod.ex/a/"),
                GraphEvent::Added("https://pod.ex/a/"),
            ],
        ] {
            let after = provenance_after(&before, events, &prov, Some(BOB));
            assert!(after.is_none(), "{events:?}: nothing changes: {after:?}");
        }
    }

    #[test]
    fn dropping_a_graph_that_is_not_stored_changes_nothing() {
        let before = store(&["https://pod.ex/docs/d0"]);
        let prov = created_by(&["https://pod.ex/docs/"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/docs/"),
            GraphEvent::Added("https://pod.ex/docs/"),
        ];
        assert!(provenance_after(&before, &events, &prov, Some(BOB)).is_none());
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
        )
        .expect("a change");
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
    fn dropping_everything_forgets_everything() {
        let before = store(&["https://pod.ex/a/x"]);
        let prov = created_by(
            &["https://pod.ex/a/", "https://pod.ex/a/x", "https://pod.ex/"],
            ALICE,
        );
        let after = provenance_after(&before, &[GraphEvent::RemovedAll], &prov, Some(BOB))
            .expect("a change");
        assert!(after.is_empty(), "forgotten: {after:?}");
        let events = [
            GraphEvent::RemovedAll,
            GraphEvent::Added("https://pod.ex/a/x"),
        ];
        assert!(
            provenance_after(&before, &events, &prov, Some(BOB)).is_none(),
            "everything back as it was"
        );
    }

    #[test]
    fn an_unusable_creator_or_none_records_nothing() {
        let before = store(&["https://pod.ex/a/x"]);
        let events = [GraphEvent::Added("https://pod.ex/b/y")];
        for creator in [Some("urn:sparq:x"), None] {
            let after = provenance_after(&before, &events, &AccessProvenance::new(), creator);
            assert!(after.is_none(), "{creator:?}: {after:?}");
        }
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
            "urn:a/",
            "urn:c/d",
        ];
        let graph = store(&names);
        let enumerated = crate::loader::resources(&graph);
        let graphs = crate::loader::ResourceGraphs::of(&graph);
        assert_eq!(graphs.all(), enumerated);
        let mut probes: Vec<String> = enumerated.iter().cloned().collect();
        probes.push("urn:c/".to_owned());
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
