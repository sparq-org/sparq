//! One holder per graph directory (`sparq_core::dir_lock`): every path that opens, writes,
//! replaces or recovers a directory holds its one lock, and every graph opened from it (its
//! named graphs, a vacuumed replacement) keeps that lock while it lives.

#![cfg(feature = "mmap")]

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq-dirlock-it-{tag}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/{s}")))
}

fn triple(s: &str) -> [Term; 3] {
    [iri(s), iri("p"), iri("o")]
}

fn has(g: &Graph, s: &str) -> bool {
    let ids: Option<Vec<_>> = triple(s).iter().map(|t| g.id_of(t)).collect();
    ids.is_some_and(|ids| g.iter_ids().any(|t| t[..] == ids[..]))
}

/// A dataset with a default graph and two named graphs, saved at `dir`.
fn save_dataset(dir: &Path) {
    let nq = "<http://ex/a> <http://ex/p> <http://ex/o> .\n\
              <http://ex/b> <http://ex/p> <http://ex/o> <http://ex/g0> .\n\
              <http://ex/c> <http://ex/p> <http://ex/o> <http://ex/g1> .\n";
    Graph::load_dataset(nq, "nquads")
        .unwrap()
        .save(dir)
        .unwrap();
}

/// The identity the marker of the store at `d` records (`root <id>` or `child <id>`).
fn store_id(d: &Path) -> String {
    let marker = std::fs::read_to_string(d.join("sparq-store")).unwrap();
    marker.trim_end().split_once(' ').unwrap().1.to_string()
}

/// Records in the store at `d` that it is `kind` staging of the store whose identity is `id`
/// (its marker then carries that identity too), as a swap does before its renames: the
/// on-disk state a crash mid-swap leaves.
fn stage(d: &Path, kind: &str, id: &str) {
    std::fs::write(d.join("sparq-store"), format!("root {id}\n")).unwrap();
    std::fs::write(d.join("sparq-staging"), format!("{kind} {id}\n")).unwrap();
}

fn would_block(r: std::io::Result<Graph>) -> bool {
    matches!(r, Err(e) if e.kind() == ErrorKind::WouldBlock)
}

/// The directory of a named graph inside a graph is its root's: opening it directly, under
/// any spelling, is refused, and so is saving or building into it.
#[test]
fn a_named_graph_directory_cannot_be_opened_on_its_own() {
    let root = scratch("child-open");
    let dir = root.join("g");
    save_dataset(&dir);
    let child = dir.join("named").join("0");
    for alias in [
        child.clone(),
        dir.join("named").join("..").join("named").join("0"),
    ] {
        let err = Graph::open(&alias)
            .err()
            .expect("a named graph's directory opened");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    }
    let err = Graph::new().save(&child).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(Graph::build_external("".as_bytes(), "ntriples", &child, 1024).is_err());
    // The root itself opens, with its named graphs.
    assert_eq!(Graph::open(&dir).unwrap().named().len(), 2);
    std::fs::remove_dir_all(&root).ok();
}

/// A store's named graphs stay in the store: none can be taken out of a directory-backed
/// graph (so no handle outlives its root's lock, swaps and freezes), and a directory-backed
/// graph never becomes a named graph of another; a frozen or read-only graph takes none. The
/// root holds the lock for its named graphs, including ones created, compacted or renumbered
/// since the open.
#[test]
fn a_named_graph_cannot_leave_its_store_or_join_another() {
    let root = scratch("child-lock");
    let dir = root.join("g");
    save_dataset(&dir);
    let other = root.join("other");
    save_dataset(&other);
    let mut g = Graph::open(&dir).unwrap();
    let before = g.named().len();
    assert!(before > 0);
    let name = g.named()[0].0.clone();
    assert!(g.take_named().is_err(), "took a store's named graphs out");
    assert!(g.remove_named(&name).is_err(), "removed a store's named graph in memory only");
    assert!(g.retain_named(|_, _| false).is_err());
    assert!(g.set_named(name.clone(), Graph::new()).is_err(), "replaced a store's named graph");
    assert!(
        g.add_named(iri("g9"), Graph::open(&other).unwrap()).is_err(),
        "another store's graph joined this store"
    );
    assert_eq!(g.named().len(), before, "a refused change changed the graph");
    // Another store's graph joins no graph, in memory or not; its lock stays with it.
    let mut mem = Graph::new();
    let held = Graph::open(&other).unwrap();
    assert!(mem.add_named(iri("x"), held).is_err(), "a store's graph joined another graph");
    drop(Graph::open(&other).expect("the refused graph was dropped with its lock"));
    // A fork is an in-memory copy, and may join an in-memory graph.
    mem.add_named(name.clone(), g.named()[0].1.fork()).unwrap();
    assert!(mem.take_named().is_ok());
    // A frozen graph takes no named graph either.
    let mut frozen = Graph::new();
    frozen.freeze_for_test("injected");
    assert!(frozen.add_named(iri("x"), Graph::new()).is_err());
    assert!(frozen.extend_named([(iri("x"), Graph::new())]).is_err());
    // Created, compacted and renumbered since the open: the root holds the lock for them.
    let fresh = g.ensure_named(&iri("g2")).unwrap();
    g.apply_delta_to_named(fresh, &[triple("d")], &[]).unwrap();
    g.compact().unwrap();
    g.drop_named_durable(&iri("g0")).unwrap();
    assert!(g.named().iter().all(|(_, child)| child.is_directory_backed()));
    assert!(would_block(Graph::open(&dir)), "the root lost its lock");
    drop(g);
    let g = Graph::open(&dir).expect("released with the root");
    assert_eq!(g.named().len(), 2, "g1 and g2 survive the drop of g0");
    std::fs::remove_dir_all(&root).ok();
}

/// A graph never saves into the directory it is open from, however the path is spelled: it
/// still maps and logs into the files a save truncates. Its files are left as they were.
#[test]
fn a_graph_does_not_save_into_its_own_directory() {
    let root = scratch("own-save");
    let dir = root.join("g");
    save_dataset(&dir);
    let mut g = Graph::open(&dir).unwrap();
    g.apply_delta(&[triple("logged")], &[]).unwrap();
    #[cfg_attr(not(unix), allow(unused_mut))] // a symbolic link is added on Unix only
    let mut spellings = vec![
        dir.clone(),
        PathBuf::from(format!("{}/", dir.display())),
        root.join("g").join("..").join("g"),
    ];
    #[cfg(unix)]
    {
        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        spellings.push(link);
    }
    for spelling in &spellings {
        assert!(g.is_backed_by(spelling), "{}", spelling.display());
        for err in [
            g.save(spelling).unwrap_err(),
            g.save_compressed(spelling).unwrap_err(),
        ] {
            assert_eq!(
                err.kind(),
                ErrorKind::InvalidInput,
                "{}: {err}",
                spelling.display()
            );
        }
    }
    // A named graph cannot save over its root either.
    let err = g.named()[0].1.save(&dir).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    // Elsewhere is fine, and the graph's own directory still holds its state.
    let copy = root.join("copy");
    assert!(!g.is_backed_by(&copy));
    g.save(&copy).unwrap();
    drop(g);
    for d in [&dir, &copy] {
        let g = Graph::open(d).unwrap();
        assert!(has(&g, "a") && has(&g, "logged"), "{}", d.display());
        assert_eq!(g.named().len(), 2, "{}", d.display());
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A graph opened through a symbolic link works on the directory it resolves to: its log,
/// recovery and swaps use that one path, so a compaction rewrites the real directory and
/// leaves the link a link.
#[cfg(unix)]
#[test]
fn a_graph_opened_through_a_link_swaps_the_real_directory() {
    let root = scratch("alias-swap");
    let dir = root.join("real");
    save_dataset(&dir);
    let link = root.join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    let mut g = Graph::open(&link).unwrap();
    assert_eq!(g.persist_dir().unwrap(), dir.canonicalize().unwrap());
    g.apply_delta(&[triple("new")], &[]).unwrap();
    g.compact().unwrap();
    assert!(
        link.symlink_metadata().unwrap().file_type().is_symlink(),
        "the link was replaced"
    );
    assert!(!root.join("link.compact-old").exists() && !root.join("link.compact-new").exists());
    drop(g);
    let g = Graph::open(&dir).unwrap();
    assert!(
        has(&g, "new"),
        "the real directory holds the compacted base"
    );
    drop(g);

    // A swap interrupted after its first rename leaves the real directory missing: the link
    // dangles and is refused rather than taken for a directory to create, while the real path
    // recovers the store.
    stage(&dir, "compact-old", &store_id(&dir));
    std::fs::rename(&dir, root.join("real.compact-old")).unwrap();
    let err = Graph::open(&link).err().expect("a dangling link opened");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let err = Graph::open_or_create(&link, &Graph::new())
        .err()
        .expect("a dangling link seeded");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "new"), "recovered by the real path");
    std::fs::remove_dir_all(&root).ok();
}

/// A store whose swap a crash interrupted (between the two renames: the directory missing,
/// the new base beside it) is recovered before `open_or_create` decides whether to seed: it
/// is opened with its data, not replaced by the seed.
#[test]
fn open_or_create_recovers_a_swap_before_seeding() {
    let root = scratch("seed-recover");
    let dir = root.join("g");
    let id = "00112233445566778899aabbccddeeff";
    for kind in ["compact-new", "compact-old"] {
        let staged = root.join(format!("g.{kind}"));
        save_dataset(&root.join("staging"));
        std::fs::rename(root.join("staging"), &staged).unwrap();
        stage(&staged, kind, id);
    }
    assert!(!dir.exists());
    let seed =
        Graph::load_str("<http://ex/seed> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    let g = Graph::open_or_create(&dir, &seed).unwrap();
    assert!(has(&g, "a"), "the swapped-in store survives");
    assert!(!has(&g, "seed"), "a store mid-swap is not re-seeded");
    assert_eq!(g.named().len(), 2);
    std::fs::remove_dir_all(&root).ok();
}

/// A restore that fails before its renames leaves the directory as it was, and the graph keeps
/// logging to it: an update acknowledged after the failure survives a reopen.
#[test]
fn a_failed_restore_in_place_keeps_logging() {
    let root = scratch("restore-fail");
    let dir = root.join("g");
    save_dataset(&dir);
    let mut g = Graph::open(&dir).unwrap();
    // A file where the swap stages the new base makes writing it fail.
    std::fs::write(root.join("g.compact-new"), b"in the way").unwrap();
    let fresh = Graph::load_str("<http://ex/r> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    assert!(g.restore_in_place(fresh).is_err());
    g.apply_delta(&[triple("after")], &[]).unwrap();
    assert!(would_block(Graph::open(&dir)), "the graph keeps its lock");
    drop(g);
    // What is in the way is not this store's staging: opening refuses it and leaves it alone
    // until it is removed by hand.
    let err = Graph::open(&dir)
        .err()
        .expect("opened past foreign staging");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(root.join("g.compact-new").is_file());
    std::fs::remove_file(root.join("g.compact-new")).unwrap();
    let g = Graph::open(&dir).unwrap();
    assert!(
        has(&g, "a") && has(&g, "after"),
        "the update after the failed restore is durable"
    );
    assert!(!has(&g, "r"));
    std::fs::remove_dir_all(&root).ok();
}

/// Read-only opens share the lock with each other and never with a writable holder; they write
/// nothing into the directory and refuse every write.
#[test]
fn read_only_opens_share_the_directory_and_exclude_a_writer() {
    let root = scratch("read-only");
    let dir = root.join("g");
    save_dataset(&dir);
    let first = Graph::open_read_only(&dir).unwrap();
    let mut second = Graph::open_read_only(&root.join("g").join("..").join("g")).unwrap();
    assert!(has(&first, "a") && has(&second, "a"));
    assert_eq!(second.named().len(), 2);
    assert!(!first.is_directory_backed());
    assert!(
        would_block(Graph::open(&dir)),
        "a writer got in beside readers"
    );
    assert!(
        Graph::new().save(&dir).is_err(),
        "a save got in beside readers"
    );
    assert!(
        !dir.join("wal.log").exists() && !dir.join("txn.log").exists(),
        "a read-only open wrote"
    );
    assert!(second.apply_delta(&[triple("x")], &[]).is_err());
    assert!(second.apply_delta_to_named(0, &[triple("x")], &[]).is_err());
    assert!(second.ensure_named(&iri("g9")).is_err());
    assert!(second.compact().is_err() && second.vacuum().is_err());
    assert_eq!(
        second.save(&dir).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    second
        .save(&root.join("copy"))
        .expect("a read-only graph saves elsewhere");
    drop((first, second));

    let mut writer = Graph::open(&dir).unwrap();
    assert!(
        would_block(Graph::open_read_only(&dir)),
        "a reader got in beside a writer"
    );
    writer.apply_delta(&[triple("logged")], &[]).unwrap();
    drop(writer);
    // A log to replay needs a writable open (and a compaction) first.
    let err = Graph::open_read_only(&dir)
        .err()
        .expect("opened read-only over a log");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    Graph::open(&dir).unwrap().compact().unwrap();
    let g = Graph::open_read_only(&dir).unwrap();
    assert!(has(&g, "logged"));
    // The directory of a named graph is refused read-only too.
    assert!(Graph::open_read_only(&dir.join("named").join("0")).is_err());
    drop(g);

    // So is the residue of an interrupted swap.
    save_dataset(&root.join("g.compact-new"));
    let err = Graph::open_read_only(&dir)
        .err()
        .expect("opened read-only over a swap's residue");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    std::fs::remove_dir_all(&root).ok();
}

/// A fork (and a snapshot) shares the base's memory-mapped files, so it holds the directory's
/// lock after the graph it came from is gone, and cannot be saved over that directory.
#[test]
fn a_fork_keeps_the_lock_and_cannot_save_over_its_source() {
    let root = scratch("fork");
    let dir = root.join("g");
    save_dataset(&dir);
    for read_only in [false, true] {
        let reader = if read_only {
            Graph::open_read_only(&dir).unwrap()
        } else {
            Graph::open(&dir).unwrap()
        };
        let fork = reader.fork();
        let snapshot = reader.snapshot();
        drop(reader);
        assert!(
            would_block(Graph::open(&dir)),
            "the lock went with the reader"
        );
        let err = fork.save(&dir).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        assert!(has(&fork, "a") && has(&snapshot, "a"));
        drop(fork);
        assert!(would_block(Graph::open(&dir)), "the snapshot keeps it too");
        drop(snapshot);
        drop(Graph::open(&dir).expect("released with the last fork"));
    }
    std::fs::remove_dir_all(&root).ok();
}

/// Ownership is marked, not inferred from names: nothing can be opened, saved or built inside a
/// store, a save into a directory refuses another store inside it, and a refused claim creates
/// no directory.
#[test]
fn a_store_owns_everything_under_it() {
    let root = scratch("ownership");
    let dir = root.join("g");
    save_dataset(&dir);
    assert!(dir.join("sparq-store").exists() && dir.join("named/0/sparq-store").exists());
    for inside in [
        dir.join("other"),
        dir.join("named").join("7"),
        dir.join("a").join("b"),
    ] {
        let err = Graph::open(&inside).err().expect("opened inside a store");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        let err = Graph::new().save(&inside).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        assert!(Graph::open_read_only(&inside).is_err());
    }
    assert!(
        !dir.join("a").exists(),
        "a refused claim created a directory"
    );

    // A store made where a root will later be saved: the save refuses to delete it.
    let later = root.join("later");
    let squatter = later.join("named").join("0");
    save_dataset(&squatter);
    let held = Graph::open(&squatter).unwrap();
    let err = Graph::load_dataset(
        "<http://ex/x> <http://ex/p> <http://ex/o> <http://ex/g> .",
        "nquads",
    )
    .unwrap()
    .save(&later)
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(has(&held, "a"));
    drop(held);
    assert!(
        Graph::open(&squatter).is_ok(),
        "the store inside was left alone"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A store saved before markers is a store all the same: refused as an ancestor, opened as
/// before, and marked on its next writable open (a read-only open writes nothing).
#[test]
fn a_legacy_store_without_markers_is_still_a_store() {
    let root = scratch("legacy");
    let dir = root.join("g");
    save_dataset(&dir);
    for d in [dir.clone(), dir.join("named/0"), dir.join("named/1")] {
        std::fs::remove_file(d.join("sparq-store")).unwrap();
    }
    assert!(Graph::open(&dir.join("named").join("0")).is_err());
    drop(Graph::open_read_only(&dir).unwrap());
    assert!(
        !dir.join("sparq-store").exists(),
        "a read-only open wrote a marker"
    );
    let g = Graph::open(&dir).unwrap();
    assert_eq!(g.named().len(), 2);
    let marker = std::fs::read_to_string(dir.join("sparq-store")).unwrap();
    assert!(marker.starts_with("root "), "{marker}");
    let marker = std::fs::read_to_string(dir.join("named/0/sparq-store")).unwrap();
    assert!(marker.starts_with("child "), "{marker}");
    std::fs::remove_dir_all(&root).ok();
}

/// A store's internals are its own: a named graph that is a symbolic link to another store, or
/// a file hard-linked with another store's, is refused (writable and read-only).
#[cfg(unix)]
#[test]
fn linked_internals_are_refused() {
    let root = scratch("links");
    let a = root.join("a");
    let b = root.join("b");
    save_dataset(&a);
    save_dataset(&b);
    // b's first named graph is a link to a's.
    std::fs::remove_dir_all(b.join("named/0")).unwrap();
    std::os::unix::fs::symlink(a.join("named/0"), b.join("named/0")).unwrap();
    for err in [Graph::open(&b).err(), Graph::open_read_only(&b).err()] {
        let err = err.expect("opened over a linked named graph");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    }
    // c shares a's write-ahead log through a hard link (a file written in place).
    let c = root.join("c");
    save_dataset(&c);
    drop(Graph::open(&a).unwrap());
    std::fs::remove_file(c.join("wal.log")).ok();
    std::fs::hard_link(a.join("wal.log"), c.join("wal.log")).unwrap();
    for d in [&a, &c] {
        let err = Graph::open(d)
            .err()
            .expect("opened over a hard-linked file");
        assert_eq!(
            err.kind(),
            ErrorKind::InvalidInput,
            "{}: {err}",
            d.display()
        );
    }
    // A file a save replaces by a rename may be hard-linked (a `cp -al` snapshot of a store):
    // it opens, and a save replaces it without writing through the link.
    let d = root.join("d");
    save_dataset(&d);
    let snapshot = root.join("snapshot-perm0.bin");
    std::fs::hard_link(d.join("perm0.bin"), &snapshot).unwrap();
    let before = std::fs::read(&snapshot).unwrap();
    drop(Graph::open(&d).expect("a hard-linked base file is not refused"));
    Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples")
        .unwrap()
        .save(&d)
        .unwrap();
    assert_eq!(std::fs::read(&snapshot).unwrap(), before, "written through the snapshot");
    std::fs::remove_dir_all(&root).ok();
}

/// A read-only graph refuses the durable clear and the rebuilding update alike.
#[test]
fn a_read_only_graph_refuses_every_mutator() {
    let root = scratch("ro-mutators");
    let dir = root.join("g");
    save_dataset(&dir);
    let mut g = Graph::open_read_only(&dir).unwrap();
    assert!(g.check_writable().is_err());
    assert!(g.clear_default_durable().is_err());
    assert!(g.clear_named_durable(&iri("g0")).is_err());
    assert!(g.drop_all_named_durable().is_err());
    assert!(has(&g, "a"), "a refused clear changed the graph");
    assert!(
        g.fork().check_writable().is_ok(),
        "a fork is an in-memory copy"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A graph rebuilt around another's dictionary (`into_dict` + `from_parts`, as a reasoner does)
/// keeps the directory locked while the mapped dictionary lives, writable or read-only.
#[test]
fn a_rebuilt_graph_keeps_its_dictionarys_lock() {
    let root = scratch("rebuilt");
    let dir = root.join("g");
    save_dataset(&dir);
    for read_only in [false, true] {
        let g = if read_only {
            Graph::open_read_only(&dir).unwrap()
        } else {
            Graph::open(&dir).unwrap()
        };
        let triples: Vec<_> = g.iter_ids().collect();
        let rebuilt = Graph::from_parts(g.into_dict(), triples);
        assert!(
            would_block(Graph::open(&dir)),
            "the lock went with the graph the dictionary came from"
        );
        assert!(has(&rebuilt, "a"));
        let err = rebuilt.save(&dir).expect_err("saved over the files it maps");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidInput,
            "a save into its own directory names the reason, it does not wait on its own lock: {err}"
        );
        drop(rebuilt);
        drop(Graph::open(&dir).expect("released with the dictionary"));
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A whole-store write (`restore_into_durable`, `save`) never replaces a directory holding
/// another store it cannot tell is its own: an independently held root, a closed legacy store
/// without a marker, or a marker that cannot be parsed. Everything inside is left as it was.
#[test]
fn a_whole_store_write_refuses_what_it_cannot_tell_is_its_own() {
    let root = scratch("foreign");
    let fresh =
        || Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();

    // An independently locked root inside the target of a restore.
    let target = root.join("t");
    let nested = target.join("sub");
    save_dataset(&nested);
    let held = Graph::open(&nested).unwrap();
    let err = Graph::restore_into_durable(&target, fresh())
        .err()
        .expect("restored over a held store");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(has(&held, "a"));
    drop(held);
    assert!(
        Graph::open(&nested).is_ok(),
        "the held store was left alone"
    );

    // A closed legacy store (no markers) where the future root's first named graph would go.
    let later = root.join("later");
    let legacy = later.join("named").join("0");
    save_dataset(&legacy);
    for d in [
        legacy.clone(),
        legacy.join("named/0"),
        legacy.join("named/1"),
    ] {
        std::fs::remove_file(d.join("sparq-store")).unwrap();
    }
    let err = fresh().save(&later).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    let err = Graph::restore_into_durable(&later, fresh())
        .err()
        .expect("restored over a legacy store");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(
        has(&Graph::open_read_only(&legacy).unwrap(), "a"),
        "the legacy store was left alone"
    );

    // A marker that is neither `root` nor `child`: refused, never guessed.
    let odd = root.join("odd");
    std::fs::create_dir_all(odd.join("x")).unwrap();
    std::fs::write(odd.join("x").join("sparq-store"), b"garbage").unwrap();
    let err = fresh().save(&odd).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(odd.join("x").join("sparq-store").exists());
    // ... and a store whose own marker is garbage does not open writable.
    let own = root.join("own");
    save_dataset(&own);
    std::fs::write(own.join("sparq-store"), b"garbage").unwrap();
    let err = Graph::open(&own)
        .err()
        .expect("opened under a garbage marker");
    assert_eq!(err.kind(), ErrorKind::InvalidData, "{err}");

    // A restore over a directory a crash left mid-swap recovers it first.
    let swapped = root.join("swapped");
    save_dataset(&swapped);
    stage(&swapped, "compact-old", &store_id(&swapped));
    std::fs::rename(&swapped, swapped.with_extension("compact-old")).unwrap();
    let g = Graph::restore_into_durable(&swapped, fresh()).unwrap();
    assert!(has(&g, "n") && !has(&g, "a"));
    assert!(!swapped.with_extension("compact-old").exists());
    std::fs::remove_dir_all(&root).ok();
}

/// A save writes every file to a fresh sibling and renames it into place: a file already there
/// is replaced, never written through (a handle on the old file still reads the old bytes), no
/// temporary file is left behind, and a directory whose files are hard-linked elsewhere is
/// refused for saves and builds alike.
#[cfg(unix)]
#[test]
fn a_save_replaces_files_and_never_writes_through_them() {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let root = scratch("fresh");
    let dir = root.join("g");
    save_dataset(&dir);
    let perm = dir.join("perm0.bin");
    let before = std::fs::read(&perm).unwrap();
    let inode = std::fs::metadata(&perm).unwrap().ino();
    let mut old = std::fs::File::open(&perm).unwrap();
    Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples")
        .unwrap()
        .save(&dir)
        .unwrap();
    assert_ne!(
        std::fs::metadata(&perm).unwrap().ino(),
        inode,
        "rewritten in place"
    );
    let mut still = Vec::new();
    old.read_to_end(&mut still).unwrap();
    assert_eq!(still, before, "the old file was written through");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().ends_with(".tmp"),
            "left {name:?} behind"
        );
    }
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "n") && !has(&g, "a"));
    drop(g);

    // A file of the store hard-linked to one outside it: a save or build replaces the store's
    // file, never writing through the link.
    let outside = root.join("outside.bin");
    std::fs::write(&outside, b"not a store file").unwrap();
    std::fs::remove_file(&perm).unwrap();
    std::fs::hard_link(&outside, &perm).unwrap();
    Graph::new().save(&dir).unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"not a store file");
    std::fs::remove_file(&perm).unwrap();
    std::fs::hard_link(&outside, &perm).unwrap();
    Graph::build_external("".as_bytes(), "ntriples", &dir, 1024).unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"not a store file");
    // The write-ahead log is written in place: a hard link to it is refused.
    std::fs::remove_file(dir.join("wal.log")).ok();
    std::fs::hard_link(&outside, dir.join("wal.log")).unwrap();
    let err = Graph::new().save(&dir).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert_eq!(std::fs::read(&outside).unwrap(), b"not a store file");
    std::fs::remove_dir_all(&root).ok();
}

/// A read-only open checks for links before it walks the store for residue, so a `named/` that
/// links back into itself is refused at once rather than walked forever.
#[cfg(unix)]
#[test]
fn a_read_only_open_does_not_walk_a_link_loop() {
    let root = scratch("loop");
    let dir = root.join("g");
    save_dataset(&dir);
    // Each of x's entries links back to x: a walk that follows links fans out without end.
    let x = root.join("x");
    for i in 0..3 {
        std::fs::create_dir_all(x.join(i.to_string())).unwrap();
        std::os::unix::fs::symlink(&x, x.join(i.to_string()).join("named")).unwrap();
    }
    std::fs::remove_dir_all(dir.join("named")).unwrap();
    std::os::unix::fs::symlink(&x, dir.join("named")).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let d = dir.clone();
    std::thread::spawn(move || {
        tx.send(Graph::open_read_only(&d).err().map(|e| e.kind()))
            .ok();
    });
    let kind = rx
        .recv_timeout(std::time::Duration::from_secs(30))
        .expect("the read-only open did not return");
    assert_eq!(kind, Some(ErrorKind::InvalidInput));
    std::fs::remove_dir_all(&root).ok();
}

/// Recovery removes or promotes only staging that records this store as its owner: another
/// store (or staging left by an older version, which records no owner) at a staging name is
/// refused by every save, build, restore and open, and left exactly as it was.
#[test]
fn recovery_never_removes_what_this_store_did_not_stage() {
    let root = scratch("foreign-staging");
    let fresh =
        || Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    let foreign = |at: &Path| {
        save_dataset(&root.join("elsewhere"));
        std::fs::rename(root.join("elsewhere"), at).unwrap();
    };
    // Checked under a neutral name (a store at a staging name reads as mid-swap), then put back.
    let intact = |at: &Path| {
        let check = root.join("check");
        std::fs::rename(at, &check).unwrap();
        assert!(
            has(&Graph::open_read_only(&check).unwrap(), "a"),
            "{} was touched",
            at.display()
        );
        std::fs::rename(&check, at).unwrap();
    };
    let refused = |what: &str, r: std::io::Result<()>| {
        let err = r
            .err()
            .unwrap_or_else(|| panic!("{what} went past foreign staging"));
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{what}: {err}");
    };

    // Beside a store with `named/`, an independent store at `named.drop-old`.
    let target = root.join("target");
    save_dataset(&target);
    let drop_old = target.join("named.drop-old");
    foreign(&drop_old);
    refused("save", fresh().save(&target));
    refused("build", {
        let nt = "<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        Graph::build_external(nt.as_bytes(), "ntriples", &target, 1 << 16)
            .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))
    });
    refused(
        "restore",
        Graph::restore_into_durable(&target, fresh()).map(drop),
    );
    refused("open", Graph::open(&target).map(drop));
    std::fs::rename(&drop_old, root.join("moved")).unwrap();
    intact(&root.join("moved"));
    std::fs::remove_dir_all(root.join("moved")).unwrap();

    // An independent store at `<target>.compact-new`, with the target present ...
    let compact_new = target.with_extension("compact-new");
    foreign(&compact_new);
    refused("save", fresh().save(&target));
    refused("open", Graph::open(&target).map(drop));
    intact(&compact_new);
    // ... and staging that names another store as its owner.
    stage(
        &compact_new,
        "compact-new",
        "ffffffffffffffffffffffffffffffff",
    );
    refused("save", fresh().save(&target));
    assert!(compact_new.join("sparq-staging").exists());
    std::fs::remove_dir_all(&compact_new).unwrap();

    // With the target missing, an independent store at `<target>.compact-old` is not taken
    // for the target's old base.
    let gone = root.join("gone");
    let compact_old = gone.with_extension("compact-old");
    foreign(&compact_old);
    refused("save", fresh().save(&gone));
    refused("open", Graph::open(&gone).map(drop));
    assert!(!gone.exists());
    intact(&compact_old);
    std::fs::remove_dir_all(&root).ok();
}

/// A build keeps its side files in a private scratch directory it makes afresh: links already
/// in the store directory (here `tmp/p<n>`, where builds used to sort, each pointing into
/// another directory holding a `run0.bin`, the name a sort's first run takes) are never
/// followed, and no scratch directory is left behind.
#[cfg(unix)]
#[test]
fn a_build_never_writes_through_a_link_in_its_directory() {
    let root = scratch("build-scratch");
    let other = root.join("other");
    let dir = root.join("g");
    std::fs::create_dir_all(dir.join("tmp")).unwrap();
    for p in 1..6 {
        let target = other.join(format!("p{p}"));
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("run0.bin"), b"keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("tmp").join(format!("p{p}"))).unwrap();
    }
    // More rows than a sibling sort holds in memory, so each one spills runs.
    let rows = 70_000;
    let nt: String = (0..rows)
        .map(|i| format!("<http://ex/s{i}> <http://ex/p{}> <http://ex/o> .\n", i % 7))
        .collect();
    Graph::build_external(nt.as_bytes(), "ntriples", &dir, 1 << 12).unwrap();
    for p in 1..6 {
        let target = other.join(format!("p{p}"));
        let names: Vec<_> = std::fs::read_dir(&target)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            names,
            ["run0.bin"],
            "the build wrote through {}",
            target.display()
        );
        assert_eq!(std::fs::read(target.join("run0.bin")).unwrap(), b"keep");
    }
    let left = std::fs::read_dir(&dir).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".sparq-")
    });
    assert!(!left, "a scratch directory was left behind");
    assert_eq!(Graph::open(&dir).unwrap().len(), rows);
    std::fs::remove_dir_all(&root).ok();
}

/// A store with only a marker (identity `id`) and the content of a saved dataset: the old or
/// new half of a swap caught between a rename and its staging record.
fn half(at: &Path, id: &str, tag: &str) {
    let nt = format!("<http://ex/{tag}> <http://ex/p> <http://ex/o> .");
    // Saved under another name and renamed: a save at a staging name beside an unrecorded half
    // is refused (it may be an earlier release's staging).
    let mut name = at.file_name().unwrap().to_os_string();
    name.push(".saving");
    let saving = at.with_file_name(name);
    Graph::load_str(&nt, "ntriples").unwrap().save(&saving).unwrap();
    std::fs::rename(&saving, at).unwrap();
    std::fs::write(at.join("sparq-store"), format!("root {id}\n")).unwrap();
}

/// Staging is per store and never a store of its own: `g.a` and `g.b` stage at
/// `g.a.compact-*` and `g.b.compact-*` (never a shared `g.compact-*`); a directory carrying a
/// staging record is refused by every claim and left as it is; staging at the names an
/// earlier version shared is removed only when it records this store.
#[test]
fn staging_is_per_store_and_never_claimed_on_its_own() {
    let root = scratch("staging-names");
    let (a, b) = (root.join("g.a"), root.join("g.b"));
    save_dataset(&a);
    save_dataset(&b);
    let mut g = Graph::open(&a).unwrap();
    g.apply_delta(&[triple("z")], &[]).unwrap();
    g.compact().unwrap();
    assert!(has(&g, "z"));
    drop(g);
    let names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    // The swap staged at `g.a.compact-*` (whose locks stay, like any lock file), never at the
    // `g.compact-*` that `g.b` would share, and left no staging directory.
    assert!(
        names.iter().any(|n| n == "g.a.compact-new.lock"),
        "the swap took no lock on its staging path: {names:?}"
    );
    assert!(
        names.iter().all(
            |n| !n.starts_with("g.compact") && (!n.contains("compact") || n.ends_with(".lock"))
        ),
        "staging left after a swap: {names:?}"
    );
    assert!(has(&Graph::open(&a).unwrap(), "z"));
    assert!(has(&Graph::open(&b).unwrap(), "a"), "g.b was touched");

    // The names an earlier release shared (`g.compact-*`) are never touched by a swap or a
    // recovery of a store that is there, recorded or not.
    let (mine, theirs) = (root.join("g.compact-old"), root.join("g.compact-new"));
    save_dataset(&mine);
    stage(&mine, "compact-old", &store_id(&a));
    save_dataset(&theirs);
    stage(&theirs, "compact-new", &store_id(&b));
    drop(Graph::open(&a).unwrap());
    assert!(mine.join("sparq-staging").exists(), "staging at a legacy name was touched");
    assert!(
        theirs.join("sparq-staging").exists(),
        "another store's staging was touched"
    );

    // A staging directory is never claimed on its own, and nothing is removed for trying.
    let refused = |what: &str, r: std::io::Result<()>| {
        let err = r
            .err()
            .unwrap_or_else(|| panic!("{what} claimed a staging directory"));
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{what}: {err}");
    };
    refused("open", Graph::open(&theirs).map(drop));
    refused("read-only open", Graph::open_read_only(&theirs).map(drop));
    refused(
        "save",
        Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples")
            .unwrap()
            .save(&theirs),
    );
    assert!(theirs.join("sparq-staging").exists());
    // Nor is it taken for the staging of a missing `g` (the store whose staging the name
    // would be now): it records `g.b`, which is there.
    refused("open g", Graph::open(&root.join("g")).map(drop));
    assert!(!root.join("g").exists() && theirs.join("sparq-staging").exists());
    assert!(has(&Graph::open(&a).unwrap(), "a") && has(&Graph::open(&b).unwrap(), "a"));
    std::fs::remove_dir_all(&root).ok();
}

/// A crash can leave one half of a swap without its staging record (the old base renamed
/// aside but not yet marked, or the new base unmarked just before its rename): recovery takes
/// that half only when its own marker carries the identity the other half's record names.
#[test]
fn recovery_takes_an_unmarked_half_only_of_the_same_store() {
    let root = scratch("unmarked-half");
    let id = "00112233445566778899aabbccddeeff";
    let other = "ffeeddccbbaa99887766554433221100";
    let swap = |name: &str, old_marked: bool, new_marked: bool, new_id: &str| {
        let (old, new) = (
            root.join(format!("{name}.compact-old")),
            root.join(format!("{name}.compact-new")),
        );
        half(&old, id, "old");
        half(&new, new_id, "new");
        if old_marked {
            std::fs::write(old.join("sparq-staging"), format!("compact-old {id}\n")).unwrap();
        }
        if new_marked {
            std::fs::write(new.join("sparq-staging"), format!("compact-new {new_id}\n")).unwrap();
        }
        (root.join(name), old, new)
    };
    // Old base aside, not yet marked: the new base is promoted.
    let (dir, old, new) = swap("p", false, true, id);
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "new") && !old.exists() && !new.exists());
    assert!(
        !dir.join("sparq-staging").exists(),
        "the live store carries a staging record"
    );
    drop(g);
    // New base unmarked just before its rename: promoted.
    let (dir, old, new) = swap("q", true, false, id);
    let g = Graph::open(&dir).unwrap();
    assert!(has(&g, "new") && !old.exists() && !new.exists());
    assert!(
        !dir.join("sparq-staging").exists(),
        "the live store carries a staging record"
    );
    drop(g);
    // An unmarked "new base" of another store: refused, both left alone.
    let (dir, old, new) = swap("r", true, false, other);
    let err = Graph::open(&dir)
        .err()
        .expect("took another store's directory");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(!dir.exists() && old.join("sparq-staging").exists() && new.exists());
    std::fs::remove_dir_all(&root).ok();
}

/// A writable open checks a store's internals before recovery touches anything: with
/// `named/` a link to a directory outside the store (here one carrying a staging record) and
/// drop staging beside it, the open is refused and the outside directory keeps its record.
#[cfg(unix)]
#[test]
fn recovery_never_follows_a_link_out_of_the_store() {
    let root = scratch("recover-link");
    let outside = root.join("A");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("sparq-staging"),
        b"drop-old 00112233445566778899aabbccddeeff\n",
    )
    .unwrap();
    let dir = root.join("B");
    save_dataset(&dir);
    let id = store_id(&dir);
    std::fs::rename(dir.join("named"), root.join("named-real")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("named")).unwrap();
    let staged = dir.join("named.drop-new");
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("sparq-staging"), format!("drop-new {id}\n")).unwrap();
    let seed = Graph::load_str("<http://ex/n> <http://ex/p> <http://ex/o> .", "ntriples").unwrap();
    for (what, r) in [
        ("open", Graph::open(&dir).map(drop)),
        (
            "open_or_create",
            Graph::open_or_create(&dir, &seed).map(drop),
        ),
    ] {
        assert!(r.is_err(), "{what} went through a linked named/");
        assert!(
            outside.join("sparq-staging").exists(),
            "{what} cleared a record outside the store"
        );
        assert!(
            staged.exists(),
            "{what} removed staging past a linked named/"
        );
    }
    std::fs::remove_dir_all(&root).ok();
}

/// A store whose parent directory is not writable (a volume mounted at the store's own path, a
/// service's state directory under a read-only parent): it has no lock file beside it, and
/// cannot get one, so its holders take the lock file inside it alone. It opens writable and
/// read-only as before, one writer at a time (and readers exclude a writer); what renames the
/// directory (a compaction swap) is refused with an error naming the parent, and the graph
/// goes on writing. Skipped where permissions are not enforced (run as root).
#[cfg(unix)]
#[test]
fn a_store_in_a_read_only_parent_opens_and_keeps_one_writer() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("ro-parent");
    let parent = root.join("mnt");
    let dir = parent.join("store");
    save_dataset(&dir);
    // As left by a version without locks: nothing beside the store.
    std::fs::remove_file(parent.join("store.lock")).unwrap();
    std::fs::remove_file(dir.join("sparq.lock")).unwrap();
    std::fs::remove_file(dir.join("sparq-store")).unwrap();
    let set_mode = |mode| {
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    set_mode(0o555);
    if std::fs::write(parent.join("probe"), b"").is_ok() {
        std::fs::remove_file(parent.join("probe")).ok();
        set_mode(0o755);
        eprintln!("skipped: permissions are not enforced here");
        std::fs::remove_dir_all(&root).ok();
        return;
    }
    let outcome = std::panic::catch_unwind(|| {
        // Read-only opens share it, and exclude a writer.
        let r1 = Graph::open_read_only(&dir).unwrap();
        let r2 = Graph::open_read_only(&dir).unwrap();
        assert!(would_block(Graph::open(&dir)), "a writer got in beside readers");
        drop((r1, r2));
        // One writer, which excludes a second writer and readers.
        let mut g = Graph::open(&dir).unwrap();
        assert!(dir.join("sparq.lock").is_file(), "upgrade did not create the inside lock");
        assert!(would_block(Graph::open(&dir)), "a second writer got in");
        assert!(would_block(Graph::open_read_only(&dir)), "a reader got in beside the writer");
        g.apply_delta(&[triple("x")], &[]).unwrap();
        let err = g.compact().unwrap_err();
        assert!(err.contains(&parent.display().to_string()), "{err}");
        let err = g.restore_in_place(Graph::new()).unwrap_err().to_string();
        assert!(err.contains(&parent.display().to_string()), "{err}");
        let slot = g.ensure_named(&iri("fresh")).unwrap();
        g.apply_delta_to_named(slot, &[triple("named-update")], &[]).unwrap();
        g.clear_named_durable(&iri("g1")).unwrap();
        g.drop_named_durable(&iri("g0")).unwrap();
        g.apply_delta(&[triple("y")], &[]).unwrap();
        drop(g);
        let g = Graph::open(&dir).unwrap();
        assert!(has(&g, "x") && has(&g, "y") && has(&g, "a"));
        assert!(has(g.named_graph(&iri("fresh")).unwrap(), "named-update"));
        assert!(g.named_graph(&iri("g0")).is_none());
        assert!(g.named_graph(&iri("g1")).unwrap().is_empty());
        drop(g);
        assert!(!parent.join("store.lock").exists());
    });
    set_mode(0o755);
    std::fs::remove_dir_all(&root).ok();
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// Opening or compacting a store that is not there fails without creating anything: no
/// parent directories, no lock file.
#[test]
fn opening_a_missing_store_creates_nothing() {
    let root = scratch("missing");
    let typo = root.join("typo").join("deeper").join("store");
    for r in [Graph::open(&typo), Graph::open_read_only(&typo)] {
        let err = r.err().expect("opened a missing store");
        assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
    }
    assert!(!root.join("typo").exists(), "made the missing parents");
    let missing = root.join("store");
    assert!(Graph::open(&missing).is_err());
    let left: Vec<_> = std::fs::read_dir(&root).unwrap().collect();
    assert!(left.is_empty(), "left {left:?}");
    std::fs::remove_dir_all(&root).ok();
}

/// Staging an earlier release left mid-swap (named with `with_extension`, so `g.a` staged at
/// `g.compact-old`, with no staging record) is refused when the store it may belong to is
/// about to be created or seeded empty: it may be that store's only copy.
#[test]
fn an_earlier_releases_staging_is_refused_before_a_store_is_created() {
    let root = scratch("legacy-staging");
    let store = root.join("g.a");
    let residue = root.join("g.compact-old");
    let saving = root.join("saving");
    save_dataset(&saving);
    std::fs::remove_file(saving.join("sparq-store")).unwrap();
    std::fs::rename(&saving, &residue).unwrap();
    let check = |what: &str, err: Option<String>| {
        let err = err.unwrap_or_else(|| panic!("{what} went ahead"));
        assert!(err.contains("g.compact-old"), "{what}: {err}");
    };
    check("open", Graph::open(&store).err().map(|e| e.to_string()));
    check(
        "open_or_create",
        Graph::open_or_create(&store, &Graph::new()).err().map(|e| e.to_string()),
    );
    check("save", Graph::new().save(&store).err().map(|e| e.to_string()));
    check(
        "build",
        Graph::build_external("".as_bytes(), "ntriples", &store, 1024).err(),
    );
    assert!(!store.join("dict-meta.bin").exists(), "seeded over the residue");
    assert!(residue.join("perm0.bin").exists(), "the residue was touched");
    // Renamed back, it is the store.
    std::fs::remove_dir_all(&store).ok();
    std::fs::rename(&residue, &store).unwrap();
    assert!(has(&Graph::open(&store).unwrap(), "a"));
    std::fs::remove_dir_all(&root).ok();
}

/// A compaction never removes what a save would refuse to replace: another store moved inside.
#[test]
fn a_compaction_refuses_a_store_moved_inside() {
    let root = scratch("compact-foreign");
    let dir = root.join("g");
    save_dataset(&dir);
    let other = root.join("other");
    save_dataset(&other);
    let mut g = Graph::open(&dir).unwrap();
    std::fs::rename(&other, dir.join("other")).unwrap();
    g.apply_delta(&[triple("z")], &[]).unwrap();
    let err = g.compact().unwrap_err();
    assert!(err.contains("another graph store"), "{err}");
    assert!(dir.join("other").join("perm0.bin").exists(), "the other store was removed");
    g.apply_delta(&[triple("w")], &[]).unwrap();
    drop(g);
    std::fs::remove_dir_all(&root).ok();
}

/// A build first removes the scratch a killed build left in the store.
#[test]
fn a_build_removes_scratch_a_killed_build_left() {
    let root = scratch("stale-scratch");
    let dir = root.join("g");
    save_dataset(&dir);
    for tag in ["build", "quads-spill"] {
        let stale = dir.join(format!(".sparq-{tag}-00112233445566778899aabbccddeeff"));
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("run-0.bin"), b"left by a killed build").unwrap();
    }
    Graph::build_external(
        "<http://ex/n> <http://ex/p> <http://ex/o> .\n".as_bytes(),
        "ntriples",
        &dir,
        1024,
    )
    .unwrap();
    let left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".sparq-"))
        .collect();
    assert!(left.is_empty(), "left {left:?}");
    assert!(has(&Graph::open(&dir).unwrap(), "n"));
    std::fs::remove_dir_all(&root).ok();
}

/// A save keeps the permissions of the files it replaces.
#[cfg(unix)]
#[test]
fn a_save_keeps_the_permissions_of_the_files_it_replaces() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("modes");
    let dir = root.join("g");
    save_dataset(&dir);
    let perm = dir.join("perm0.bin");
    std::fs::set_permissions(&perm, std::fs::Permissions::from_mode(0o640)).unwrap();
    save_dataset(&dir);
    assert_eq!(
        std::fs::metadata(&perm).unwrap().permissions().mode() & 0o777,
        0o640
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A read-only open of a store with a named graph caught mid-swap (its directory renamed
/// aside) says so, rather than that the named graph is not found.
#[test]
fn a_read_only_open_names_a_named_graph_caught_mid_swap() {
    let root = scratch("ro-named-swap");
    let dir = root.join("g");
    save_dataset(&dir);
    let named = dir.join("named").join("0");
    let aside = dir.join("named").join("0.compact-old");
    std::fs::rename(&named, &aside).unwrap();
    let id = store_id(&aside);
    std::fs::write(aside.join("sparq-staging"), format!("compact-old {id}\n")).unwrap();
    let err = Graph::open_read_only(&dir).err().expect("opened mid-swap");
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(err.to_string().contains("interrupted swap of a named graph"), "{err}");
    // A writable open recovers it.
    assert!(has(sub_graph(&Graph::open(&dir).unwrap(), "g0"), "b"));
    std::fs::remove_dir_all(&root).ok();
}

/// Reserved named-staging suffixes are recognized in every case, even on a
/// case-sensitive filesystem and when the live named graphs are still present.
#[test]
fn a_read_only_open_refuses_mixed_case_named_staging() {
    let root = scratch("ro-named-staging-case");
    let dir = root.join("g");
    save_dataset(&dir);
    for suffix in ["CoMpAcT-NeW", "COMPACT-OLD", "SpArQ-PaRt"] {
        let staging = dir.join("named").join(format!("0.{suffix}"));
        std::fs::create_dir(&staging).unwrap();
        let err = Graph::open_read_only(&dir).err().expect("opened with named staging");
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
        assert!(err.to_string().contains("interrupted swap of a named graph"), "{err}");
        assert!(staging.is_dir(), "read-only open changed staging");
        std::fs::remove_dir(staging).unwrap();
    }
    let g = Graph::open_read_only(&dir).unwrap();
    assert!(has(sub_graph(&g, "g0"), "b"));
    drop(g);
    std::fs::remove_dir_all(root).unwrap();
}

fn sub_graph<'a>(g: &'a Graph, name: &str) -> &'a Graph {
    g.named_graph(&iri(name)).unwrap()
}

/// What a tool's output may not replace is matched as the filesystem compares names: in any
/// case (`data.LOCK` for the lock of `data`), and any regular file some holder has locked now.
#[test]
fn reserved_names_match_in_any_case_and_held_files_are_reserved() {
    use sparq_core::dir_lock::is_reserved_store_path;
    let root = scratch("reserved-case");
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::create_dir_all(root.join("gK")).unwrap();
    for name in [
        "ſparq-ſtore", "ſparq-stagıng", "data.locK", "gK.LOCK",
        "data.LOCK", "data.Lock", "SPARQ-STORE", "SPARQ-STAGING", "Sparq.Lock",
        "x.Compact-New", "x.COMPACT-OLD", "named.Drop-New", "named.DROP-OLD",
        ".SPARQ-build-1", "x.SPARQ-PART", "x.COMPACT-NEW.LOCK",
    ] {
        assert!(is_reserved_store_path(&root.join(name)).unwrap(), "{name}");
    }
    // Windows drops trailing dots and spaces from a name: `data.lock.` is `data.lock` there.
    #[cfg(windows)]
    for name in ["data.lock.", "data.LOCK .", "sparq-store."] {
        assert!(is_reserved_store_path(&root.join(name)).unwrap(), "{name}");
    }
    assert!(!is_reserved_store_path(&root.join("out.nt")).unwrap());
    // A store's lock file, under any name, while its store is open.
    let dir = root.join("g");
    save_dataset(&dir);
    let renamed_lock = root.join("held-file");
    let g = Graph::open(&dir).unwrap();
    std::fs::hard_link(root.join("g.lock"), &renamed_lock).unwrap();
    assert!(is_reserved_store_path(&renamed_lock).unwrap(), "a held lock file is not reserved");
    drop(g);
    assert!(!is_reserved_store_path(&renamed_lock).unwrap());
    std::fs::remove_dir_all(&root).ok();
}

/// Special files are rejected before any store or lock file is opened or created.
#[cfg(unix)]
#[test]
fn special_files_are_neither_store_directories_nor_lock_targets() {
    use sparq_core::dir_lock::{DirLock, LockLocation};
    let root = scratch("special-files");
    let fifo = root.join("fifo");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    for path in [&fifo, Path::new("/dev/null"), Path::new("/dev/stdout")] {
        assert!(LockLocation::sibling_of(path).is_err(), "{} was accepted as a directory", path.display());
        assert!(Graph::open(path).is_err());
        assert!(Graph::open_read_only(path).is_err());
        assert!(Graph::open_or_create(path, &Graph::new()).is_err());
        assert!(Graph::new().save(path).is_err());
        assert!(Graph::build_external("".as_bytes(), "ntriples", path, 1024).is_err());
    }
    assert!(!root.join("fifo.lock").exists());
    let dir = root.join("store");
    std::os::unix::fs::symlink("/dev/null", root.join("store.lock")).unwrap();
    let location = LockLocation::sibling_of(&dir).unwrap();
    assert_eq!(DirLock::acquire(location).unwrap_err().kind(), ErrorKind::InvalidInput);
    assert!(Graph::open_or_create(&dir, &Graph::new()).is_err());
    assert!(!dir.exists());
    std::fs::remove_file(root.join("store.lock")).unwrap();
    save_dataset(&dir);
    std::fs::remove_file(dir.join("sparq.lock")).unwrap();
    std::os::unix::fs::symlink("/dev/null", dir.join("sparq.lock")).unwrap();
    assert_eq!(Graph::open(&dir).err().unwrap().kind(), ErrorKind::InvalidInput);
    std::fs::remove_dir_all(root).unwrap();
}

/// Even a shared holder must acquire the inside lock rather than proceed without one.
#[cfg(unix)]
#[test]
fn a_read_only_open_requires_access_to_the_inside_lock() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("inside-required");
    let dir = root.join("g");
    save_dataset(&dir);
    std::fs::remove_file(dir.join("sparq.lock")).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = Graph::open_read_only(&dir);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result.err().expect("opened without the inside lock").kind(), ErrorKind::PermissionDenied);
    assert!(Graph::open_read_only(&dir).is_ok());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn staging_names_in_another_case_are_not_opened_as_live_stores() {
    let root = scratch("staging-case");
    let dir = root.join("g");
    save_dataset(&dir);
    let old = root.join("g.CoMpAcT-OlD");
    let id = store_id(&dir);
    std::fs::rename(&dir, &old).unwrap();
    stage(&old, "compact-old", &id);
    for result in [Graph::open(&old), Graph::open_read_only(&old)] {
        assert_eq!(result.err().expect("opened staging as a live store").kind(), ErrorKind::InvalidInput);
    }
    assert!(old.join("sparq-staging").is_file());
    assert!(!dir.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_store_write_refuses_a_held_lock_in_another_case() {
    for name in ["child.LOCK", "SPARQ.LOCK"] {
        let root = scratch(&format!("nested-lock-case-{name}"));
        let dir = root.join("g");
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join(name);
        let held = std::fs::File::create(&path).unwrap();
        fs4::FileExt::try_lock(&held).unwrap();
        assert!(Graph::new().save(&dir).is_err(), "{name}");
        assert!(!dir.join("dict-meta.bin").exists());
        drop(held);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn store_paths_reject_regular_files_before_creating_locks() {
    let root = scratch("store-file");
    let path = root.join("ordinary-file");
    std::fs::write(&path, b"keep").unwrap();
    assert!(sparq_core::dir_lock::LockLocation::sibling_of(&path).is_err());
    assert_eq!(Graph::open_or_create(&path, &Graph::new()).err().unwrap().kind(), ErrorKind::InvalidInput);
    assert!(!root.join("ordinary-file.lock").exists());
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    std::fs::remove_dir_all(root).unwrap();
}
