//! How the trusted creator/owner provenance follows a change to the stored graphs.
//!
//! Every change the store makes to which graphs exist (an update's effects, or a caller's
//! direct edit such as the MCP server's) is replayed, in order, against one rule for which
//! resources exist ([`ResourceGraphs`], the loader's own enumeration), and only its net
//! effect counts. A resource (a document or a container) keeps its creator and owner across
//! a change if it exists before and after it, or if its own access control resource
//! (`<r>.acr`, the only document the ACP view, their one reader, reads) is stored before the
//! change, and when the provenance was last installed, and not removed at any point in it (replacing its content, by a PUT or `CLEAR` and
//! `INSERT`, is no removal; a `DROP` is); any other resource loses them. So the facts of
//! a resource that goes are kept with its `.acr` and come back with the resource only while
//! that `.acr` stays stored, and a container left with no graph below it ends like a
//! document. A resource that exists after the change and did not before gets the acting
//! agent, when there is one (a SPARQL update records none), as its creator
//! unless its own `.acr` was stored before the change. The view states creator and owner
//! facts for existing resources only, so a change that brings back or ends a resource with
//! known facts asks for a rebuild. The result is staged with the change and installed only
//! once the change commits; every rebuild also drops the facts of a resource that does not
//! exist unless its `.acr` is stored and was at the previous install. So an `.acr` stored by
//! an edit the store was not told of keeps nothing.
//!
//! Which resources the acting agent is recorded for is the caller's to say (`created`): only
//! those it names, among the ones the change brings into being. A resource stored below
//! containers that do not exist yet brings each of those containers into being with it, so a
//! caller that names the resource alone leaves them with no creator, and a writer never
//! becomes the creator of a container above what it wrote. Naming no resource records nobody.
//! A caller passing no list at all has every resource the change brings into being recorded,
//! which is what the whole-store rebuild after an update needs; the update path records no
//! creator, so only a direct edit reaches the recording at all.

use crate::loader::{validate_principal_iri, ResourceGraphs, ACR_SUFFIX};
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
            .named()
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
        .named()
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
    installed_acrs: &FxHashSet<String>,
) -> Option<AccessProvenance> {
    if events.is_empty() {
        return None;
    }
    let stored = |name: &str| {
        before
            .named()
            .iter()
            .any(|(n, _)| matches!(n, Term::NamedNode(n) if n.as_str() == name))
    };
    replay(
        ResourceGraphs::of(before),
        events,
        provenance,
        creator,
        None,
        stored,
        installed_acrs,
    )
}

/// [`provenance_after`] from the names of the graphs stored before the events, recording
/// `creator` for the resources `created` names among those the events bring into being
/// (`None` for every one of them).
pub(crate) fn provenance_after_names(
    before: &FxHashSet<String>,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
    created: Option<&[&str]>,
    installed_acrs: &FxHashSet<String>,
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
        created,
        stored,
        installed_acrs,
    )
}

fn replay(
    mut graphs: ResourceGraphs,
    events: &[GraphEvent<'_>],
    provenance: &AccessProvenance,
    creator: Option<&str>,
    created: Option<&[&str]>,
    stored_before: impl Fn(&str) -> bool,
    installed_acrs: &FxHashSet<String>,
) -> Option<AccessProvenance> {
    // Each resource the change touches, with whether it was one before the change (the first
    // time the change touches it says which). Only the net effect counts: a resource that
    // exists before and after keeps its creator and owner, whatever happens in between.
    let mut touched: FxHashMap<String, bool> = FxHashMap::default();
    // The access control resources the change removes at any point, and whether it removes
    // every graph: one removed and stored again is not stored throughout.
    let mut acr_removed: FxHashSet<&str> = FxHashSet::default();
    let mut cleared = false;
    for event in events {
        match *event {
            GraphEvent::Added(name) => {
                graphs.add(name, |r| {
                    touched.entry(r.to_owned()).or_insert(false);
                });
            }
            GraphEvent::Removed(name) => {
                if name.ends_with(ACR_SUFFIX) {
                    acr_removed.insert(name);
                }
                graphs.remove(name, |r| {
                    touched.entry(r.to_owned()).or_insert(true);
                });
            }
            GraphEvent::RemovedAll => {
                cleared = true;
                graphs.clear(|r| {
                    touched.entry(r.to_owned()).or_insert(true);
                });
            }
        }
    }
    // Only the access control resource counts: the ACP view, the one reader of creators and
    // owners, reads no `.acl`.
    let acr_of = |r: &str| format!("{r}{ACR_SUFFIX}");
    // A resource's own `.acr` stored before the change, and when the provenance was last
    // installed (so one an edit the store was not told of stored keeps nothing), and never
    // removed in the change.
    let acr_throughout = |r: &str| {
        let acr = acr_of(r);
        !cleared
            && installed_acrs.contains(r)
            && stored_before(&acr)
            && !acr_removed.contains(acr.as_str())
    };
    let existed = |r: &str| touched.get(r).copied().unwrap_or_else(|| graphs.holds(r));
    let creator = creator.filter(|c| validate_principal_iri(c).is_ok());
    // A resource keeps its creator and owner across the change if it exists before and after
    // it, or its own `.acr` is stored throughout it; any other loses them.
    let mut gone: Vec<&str> = Vec::new();
    // Whether a resource whose creator or owner is known starts or stops existing: the view
    // states those facts for existing resources only, so it must be rebuilt even when the
    // provenance itself stays as it is.
    let mut known_moved = false;
    for (r, _, _) in provenance.iter() {
        let (was, now) = (existed(r), graphs.holds(r));
        known_moved |= was != now;
        if !(was && now) && !acr_throughout(r) {
            gone.push(r);
        }
    }
    // Whoever brings a resource into being is its creator, unless its own `.acr` was stored
    // before the change. A caller that names the resources it creates (`created`) limits the
    // recording to them, so the containers a write brings into being alongside the resource it
    // names keep no creator.
    let named = |r: &str| created.is_none_or(|only| only.contains(&r));
    let began: Vec<&str> = match creator {
        Some(_) => touched
            .iter()
            .filter(|(r, was)| {
                !**was && graphs.holds(r) && !stored_before(&acr_of(r)) && named(r)
            })
            .map(|(r, _)| r.as_str())
            .collect(),
        None => Vec::new(),
    };
    if gone.is_empty() && began.is_empty() {
        return known_moved.then(|| provenance.clone());
    }
    let mut kept = provenance.clone();
    for r in gone {
        kept.forget(r);
    }
    if let Some(creator) = creator {
        for r in began {
            kept.set_creator(r, creator);
        }
    }
    Some(kept)
}

#[cfg(test)]
mod tests {
    use super::GraphEvent;
    use crate::AccessProvenance;
    use sparq_core::Graph;

    const ALICE: &str = "https://alice.ex/card#me";
    const BOB: &str = "https://bob.ex/card#me";

    /// [`super::provenance_after`] with every `.acr` stored before the change stored when the
    /// provenance was installed.
    fn provenance_after(
        before: &Graph,
        events: &[GraphEvent<'_>],
        provenance: &AccessProvenance,
        creator: Option<&str>,
    ) -> Option<AccessProvenance> {
        let installed = crate::loader::controlled_resources(before)
            .into_iter()
            .map(str::to_owned)
            .collect();
        super::provenance_after(before, events, provenance, creator, &installed)
    }

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
            Some(BOB),
            "a .acl is not what the ACP view reads"
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
            "/a/ and its .acr went"
        );
        let all = [GraphEvent::RemovedAll];
        let after = provenance_after(&before, &all, &prov, Some(BOB)).expect("a change");
        assert!(after.is_empty(), "everything went: {after:?}");
    }

    #[test]
    fn a_container_ends_like_any_resource_unless_its_acr_is_stored_throughout() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/b/x", "https://pod.ex/b/.acr"]);
        let prov = created_by(&["https://pod.ex/a/", "https://pod.ex/b/"], ALICE);
        let emptied = [
            GraphEvent::Removed("https://pod.ex/a/x"),
            GraphEvent::Removed("https://pod.ex/b/x"),
        ];
        let after = provenance_after(&before, &emptied, &prov, Some(BOB)).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/"), None, "/a/ is gone");
        assert_eq!(
            after.creator("https://pod.ex/b/"),
            Some(ALICE),
            "kept with /b/.acr"
        );
        let back = [
            GraphEvent::Added("https://pod.ex/a/y"),
            GraphEvent::Added("https://pod.ex/b/y"),
        ];
        let before_back = store(&["https://pod.ex/b/.acr"]);
        let again = provenance_after(&before_back, &back, &after, Some(BOB)).expect("a change");
        assert_eq!(again.creator("https://pod.ex/a/"), Some(BOB), "created again");
        assert_eq!(again.creator("https://pod.ex/b/"), Some(ALICE), "back with it");
    }

    #[test]
    fn a_change_that_moves_a_resource_with_known_facts_asks_for_a_rebuild() {
        let before = store(&["https://pod.ex/x", "https://pod.ex/x.acr"]);
        let prov = created_by(&["https://pod.ex/x"], ALICE);
        let gone = [GraphEvent::Removed("https://pod.ex/x")];
        let after = provenance_after(&before, &gone, &prov, None).expect("a rebuild");
        assert_eq!(after, prov, "kept with x.acr");
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
    fn an_access_control_list_keeps_no_creator() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/a/x.acl"]);
        let prov = created_by(&["https://pod.ex/a/x"], ALICE);
        let gone = [GraphEvent::Removed("https://pod.ex/a/x")];
        let after = provenance_after(&before, &gone, &prov, None).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/x"), None);
    }

    #[test]
    fn a_resource_back_without_its_access_control_resource_drops_what_was_kept() {
        let before = store(&["https://pod.ex/a/x.acr"]);
        let prov = created_by(&["https://pod.ex/a/x"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x.acr"),
            GraphEvent::Added("https://pod.ex/a/x"),
        ];
        let after = provenance_after(&before, &events, &prov, None).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/x"), None, "no update records a creator");
        let after = provenance_after(&before, &events, &prov, Some(BOB)).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/x"), None, "its document predates it");
        let fresh = store(&[]);
        let events = [GraphEvent::Added("https://pod.ex/a/x")];
        let after = provenance_after(&fresh, &events, &prov, Some(BOB)).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/x"), Some(BOB), "the old entry goes");
    }

    #[test]
    fn an_access_control_resource_removed_and_stored_again_keeps_nothing() {
        let before = store(&["https://pod.ex/a/x", "https://pod.ex/a/x.acr"]);
        let prov = created_by(&["https://pod.ex/a/x"], ALICE);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x.acr"),
            GraphEvent::Added("https://pod.ex/a/x.acr"),
            GraphEvent::Removed("https://pod.ex/a/x"),
        ];
        let after = provenance_after(&before, &events, &prov, None).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/x"), None, "not stored throughout");
        let absent = store(&["https://pod.ex/a/x.acr"]);
        let events = [
            GraphEvent::Removed("https://pod.ex/a/x.acr"),
            GraphEvent::Added("https://pod.ex/a/x.acr"),
        ];
        let after = provenance_after(&absent, &events, &prov, None).expect("a change");
        assert_eq!(after.creator("https://pod.ex/a/x"), None, "nor while x is absent");
    }

    #[test]
    fn removing_every_graph_ends_every_container() {
        let before = store(&["https://pod.ex/c/z"]);
        let prov = created_by(&["https://pod.ex/b/"], ALICE);
        let all = [GraphEvent::RemovedAll];
        let after = provenance_after(&before, &all, &prov, None).expect("a change");
        assert_eq!(after.creator("https://pod.ex/b/"), None, "already emptied");
        let back = [GraphEvent::RemovedAll, GraphEvent::Added("https://pod.ex/b/y")];
        let after = provenance_after(&before, &back, &prov, Some(BOB)).expect("a change");
        assert_eq!(after.creator("https://pod.ex/b/"), Some(BOB), "created again");
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
