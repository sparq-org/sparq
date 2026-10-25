//! (sq-glw2) End-to-end DURABILITY test for named-graph CLEAR / DROP through
//! `update_in_place` on a DIRECTORY-BACKED store: the "204 ⇒ durable across restart" guarantee.
//!
//! The pre-fix in-place path emptied a named graph with `entry.1 = empty_graph()` — which on a
//! directory-backed parent DROPPED the sub-graph's WAL/dir association, so the clear became
//! durable only at the next compaction — and dropped one with `graph.named.retain(...)`, which
//! left the on-disk `named/<i>/` sub-dir + manifest entry behind, so a reopen RESURRECTED the
//! dropped graph. Both broke the durability contract that default-graph CLEAR
//! (`clear_default_durable`) and named-graph INSERT/DELETE (`ensure_named` + WAL) already met.
//!
//! These tests build a real directory-backed graph (`save` then `open`, the out-of-core path,
//! mmap from the sparq-core dev-dep), run the update, then SIMULATE A PROCESS RESTART by dropping
//! the in-memory graph and re-`open`ing the directory, and assert the post-restart state. Mirrors
//! `named_graph_persistence.rs` (the sq-7cxr/gh-45 persistence test) and is the engine-layer twin
//! of sparq-core's `clear_named_durable_survives_reopen` / `drop_named_durable_*` unit tests.

#![cfg(feature = "parallel")] // engine default; mmap comes from the sparq-core dev-dep

use sparq_core::Graph;
use sparq_engine::update_in_place;

/// Triple count in a (sub-)graph via a full overlay-merged scan.
fn rows(g: &Graph) -> usize {
    g.store.scan(&[None, None, None]).rows.len()
}

/// The sub-graph whose name contains `frag`, if present.
fn named<'a>(g: &'a Graph, frag: &str) -> Option<&'a Graph> {
    g.named
        .iter()
        .find(|(n, _)| n.to_string().contains(frag))
        .map(|(_, sub)| sub)
}

/// A unique temp dir per test, cleaned on entry.
fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq_glw2_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

/// (1) CLEAR GRAPH <g>: INSERT into a named graph, CLEAR it, simulate restart (reload from the
/// dir) — the named graph must be PRESENT-but-EMPTY (slot preserved, contents durably gone).
#[test]
fn clear_named_graph_survives_restart_present_but_empty() {
    let dir = tmp("clear");
    Graph::load_dataset(
        "<http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .",
        "nquads",
    )
    .unwrap()
    .save(&dir)
    .unwrap();

    {
        let mut g = Graph::open(&dir).unwrap();
        update_in_place(
            &mut g,
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :g1 { :a :b :c . :d :e :f } }",
        )
        .unwrap();
        assert_eq!(
            rows(named(&g, "g1").expect("g1 present")),
            3,
            "3 triples before CLEAR"
        );
        update_in_place(&mut g, "PREFIX : <http://ex/> CLEAR GRAPH :g1").unwrap();
        assert_eq!(
            rows(named(&g, "g1").expect("slot kept")),
            0,
            "CLEAR empties in memory"
        );
    } // drop closes the WAL/mmaps — on-disk state is the durability boundary.

    // Restart: reopen from the directory. The cleared state must persist.
    let g2 = Graph::open(&dir).unwrap();
    let g1 = named(&g2, "g1").expect("CLEAR GRAPH must PRESERVE the (now empty) slot");
    assert_eq!(
        rows(g1),
        0,
        "CLEAR GRAPH must be durable: empty after restart, not resurrected"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// (2) DROP GRAPH <g>: INSERT into a named graph, DROP it, reload — the graph must be GONE
/// (sub-dir + manifest entry removed durably), while a sibling named graph is preserved.
#[test]
fn drop_named_graph_survives_restart_gone() {
    let dir = tmp("drop");
    Graph::load_dataset(
        "<http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .\n\
         <http://ex/m> <http://ex/n> <http://ex/o> <http://ex/keep> .",
        "nquads",
    )
    .unwrap()
    .save(&dir)
    .unwrap();

    {
        let mut g = Graph::open(&dir).unwrap();
        update_in_place(
            &mut g,
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :g1 { :a :b :c } }",
        )
        .unwrap();
        assert!(named(&g, "g1").is_some() && named(&g, "keep").is_some());
        update_in_place(&mut g, "PREFIX : <http://ex/> DROP GRAPH :g1").unwrap();
        assert!(
            named(&g, "g1").is_none(),
            "DROP removes the entry in memory"
        );
        assert!(named(&g, "keep").is_some(), "sibling preserved");
    }

    // Restart: the dropped graph must NOT come back; the sibling must still be intact.
    let g2 = Graph::open(&dir).unwrap();
    assert!(
        named(&g2, "g1").is_none(),
        "DROP GRAPH must be durable: gone after restart, not resurrected"
    );
    let keep = named(&g2, "keep").expect("the un-dropped sibling must survive the restart");
    assert_eq!(
        rows(keep),
        1,
        "sibling's triple intact after the drop+restart"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// DROP of a middle graph keeps survivor slots and WAL backing stable across restart.
#[test]
fn drop_middle_named_graph_preserves_slots_and_survives_restart() {
    let dir = tmp("stable_slots");
    Graph::load_dataset(
        "<http://ex/a1> <http://ex/p> <http://ex/o> <http://ex/g1> .\n\
         <http://ex/a2> <http://ex/p> <http://ex/o> <http://ex/g2> .\n\
         <http://ex/a3> <http://ex/p> <http://ex/o> <http://ex/g3> .",
        "nquads",
    )
    .unwrap()
    .save(&dir)
    .unwrap();

    {
        let mut g = Graph::open(&dir).unwrap();
        assert_eq!(g.named.len(), 3);
        update_in_place(&mut g, "PREFIX : <http://ex/> DROP GRAPH :g2").unwrap();
        assert_eq!(g.named.len(), 2);
        assert!(named(&g, "g2").is_none());
        // Mutate a survivor after deletion to prove its WAL still targets the same slot.
        update_in_place(
            &mut g,
            "PREFIX : <http://ex/> INSERT DATA { GRAPH :g3 { :extra :p :o } }",
        )
        .unwrap();
        assert_eq!(rows(named(&g, "g3").expect("g3 present")), 2);
    }

    let g2 = Graph::open(&dir).unwrap();
    assert_eq!(g2.named.len(), 2, "exactly the two survivors after restart");
    assert!(
        named(&g2, "g2").is_none(),
        "dropped middle graph stays gone"
    );
    assert_eq!(rows(named(&g2, "g1").expect("g1 survives")), 1);
    assert_eq!(
        rows(named(&g2, "g3").expect("g3 survives")),
        2,
        "g3 keeps its original + post-drop triple across the restart"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// (3) SILENT vs non-SILENT CLEAR/DROP of a MISSING named graph keeps the existing
/// auto-create-store semantics: a no-op SUCCESS either way, and the directory-backed store
/// survives the no-op + restart unchanged. (SILENT is a no-op distinction for CLEAR/DROP here —
/// see `update::update_contract`.)
#[test]
fn clear_drop_missing_named_graph_is_noop_success_with_and_without_silent() {
    let dir = tmp("missing");
    Graph::load_dataset(
        "<http://ex/x> <http://ex/q> <http://ex/y> <http://ex/g1> .",
        "nquads",
    )
    .unwrap()
    .save(&dir)
    .unwrap();

    {
        let mut g = Graph::open(&dir).unwrap();
        for op in [
            "CLEAR GRAPH <http://ex/absent>",
            "CLEAR SILENT GRAPH <http://ex/absent>",
            "DROP GRAPH <http://ex/absent>",
            "DROP SILENT GRAPH <http://ex/absent>",
        ] {
            update_in_place(&mut g, op)
                .unwrap_or_else(|e| panic!("{op} must be a no-op success, got: {e}"));
        }
        assert_eq!(rows(named(&g, "g1").expect("g1 untouched")), 1);
    }
    let g2 = Graph::open(&dir).unwrap();
    assert_eq!(
        rows(named(&g2, "g1").expect("g1 survives")),
        1,
        "no-op clear/drop of an absent graph leaves the store intact"
    );
    assert_eq!(g2.named.len(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// (4) The IN-MEMORY parent path is unchanged: named-graph CLEAR keeps the (empty) slot, DROP
/// removes the entry, with no directory/WAL involved — byte-for-byte the previous behaviour.
#[test]
fn in_memory_parent_clear_keeps_slot_drop_removes_entry() {
    let mut g = Graph::load_dataset(
        "<http://ex/a> <http://ex/p> <http://ex/b> <http://ex/g1> .\n\
         <http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g2> .",
        "nquads",
    )
    .unwrap();
    assert_eq!(g.named.len(), 2);

    update_in_place(&mut g, "PREFIX : <http://ex/> CLEAR GRAPH :g1").unwrap();
    assert_eq!(g.named.len(), 2, "in-memory CLEAR keeps the slot");
    assert_eq!(
        rows(named(&g, "g1").expect("g1 slot kept")),
        0,
        "in-memory CLEAR empties it"
    );

    update_in_place(&mut g, "PREFIX : <http://ex/> DROP GRAPH :g2").unwrap();
    assert!(
        named(&g, "g2").is_none(),
        "in-memory DROP removes the entry"
    );
    assert_eq!(g.named.len(), 1);

    // CLEAR/DROP of a missing graph is a no-op success in memory too.
    update_in_place(&mut g, "PREFIX : <http://ex/> CLEAR GRAPH :nope").unwrap();
    update_in_place(&mut g, "PREFIX : <http://ex/> DROP GRAPH :nope").unwrap();
    assert_eq!(g.named.len(), 1);
}

/// CLEAR ALL / DROP ALL on a directory-backed store: every named graph is durably emptied
/// (CLEAR) or removed (DROP), surviving a restart.
#[test]
fn clear_all_then_drop_all_named_are_durable() {
    let dir = tmp("all");
    Graph::load_dataset(
        "<http://ex/d> <http://ex/p> <http://ex/e> .\n\
         <http://ex/a> <http://ex/p> <http://ex/o> <http://ex/g1> .\n\
         <http://ex/b> <http://ex/p> <http://ex/o> <http://ex/g2> .",
        "nquads",
    )
    .unwrap()
    .save(&dir)
    .unwrap();

    {
        let mut g = Graph::open(&dir).unwrap();
        // CLEAR ALL empties the default graph AND every named graph (slots kept).
        update_in_place(&mut g, "CLEAR ALL").unwrap();
        assert_eq!(rows(&g), 0);
        assert_eq!(
            g.named.len(),
            2,
            "CLEAR ALL keeps the (now empty) named slots"
        );
        assert!(g.named.iter().all(|(_, s)| rows(s) == 0));
    }
    {
        // Reopen: the cleared state is durable.
        let mut g = Graph::open(&dir).unwrap();
        assert_eq!(g.named.len(), 2);
        assert!(
            g.named.iter().all(|(_, s)| rows(s) == 0),
            "CLEAR ALL durable across restart"
        );
        // Now DROP ALL: removes the default-graph content AND every named graph entry.
        update_in_place(&mut g, "DROP ALL").unwrap();
        assert_eq!(g.named.len(), 0);
    }
    let g3 = Graph::open(&dir).unwrap();
    assert_eq!(
        g3.named.len(),
        0,
        "DROP ALL removes every named graph durably"
    );
    std::fs::remove_dir_all(&dir).ok();
}
